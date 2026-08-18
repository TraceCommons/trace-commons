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
    var body: some View {
        ScrollView {
            SettingsContent()
        }
        .tcScreen()
    }
}

/// The screen's content, split out of its `ScrollView` for the same reason
/// `QueueContent` and `ConsentScopesContent` are: `ImageRenderer` renders a
/// `ScrollView` as blank, so the screenshot hook can only rasterize what
/// lives outside one -- and the local change log at the foot of this screen
/// is a surface that has to be looked at to be checked.
struct SettingsContent: View {
    @EnvironmentObject private var model: AppModel
    @ObservedObject private var updates = UpdateController.shared

    // Read fresh on appear rather than cached across the view's lifetime:
    // the user can flip this in System Settings -> General -> Login Items
    // while this window is open, and a value captured once at init would
    // then claim a state that is no longer true. See `LoginItemManager`.
    @State private var loginItemState: LoginItemManager.State = LoginItemManager.currentState
    @State private var loginItemActionError: String?
    @State private var showingGoPublic = false
    /// The panel's two editable fields. Seeded from the daemon's answer --
    /// see `seedProfileDraft` -- rather than bound straight to it, so a
    /// background refresh cannot rewrite what is being typed.
    @State private var handleDraft = ""
    @State private var bioDraft = ""

    /// Spec §5.4: the Settings content column is `max-width:520px` ("prose
    /// column, kept narrow on purpose"), narrower than the 660 that
    /// `TC.Measure.prose` carries for onboarding. There is no token for it,
    /// so it is stated here rather than widening a shared one.
    private static let proseColumn: CGFloat = 520

    var body: some View {
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
            audit
        }
        .padding(.top, TC.Space.Content.top)
        .padding(.horizontal, TC.Space.Content.horizontal)
        .padding(.bottom, TC.Space.Content.bottom)
        .tcColumn(Self.proseColumn)
        .onAppear {
            loginItemState = LoginItemManager.currentState
            // Same reason the login-item state is read fresh here: the log
            // can have grown since launch (the CLI writes to it too), and
            // the daemon publishes no event when it does.
            model.refreshAudit()
        }
        .sheet(isPresented: $showingGoPublic) {
            // Handed the model explicitly rather than relying on the sheet
            // inheriting it: the dialog now makes a daemon call, and an
            // environment object it did not get would be a crash on the one
            // button that matters.
            GoPublicDialog(onDismiss: { showingGoPublic = false })
                .environmentObject(model)
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

    // Whether this contributor is on the roster used to be inferred from
    // the granted scope list. It is now read from `get_public_profile`,
    // which is the only thing that knows: `public_attribution` is a
    // permission to be listed, and claiming a handle is the separate act
    // that actually puts a row on the roster.

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
                // The door into the community-brand surface used to be
                // here, gated on this scope list. It has moved to the
                // public-profile section below, because the roster is not
                // what this list describes: the daemon deliberately does
                // not pre-check `consent_scopes` before a claim -- the
                // local list can be narrower than what the credential
                // carries, and refusing here would refuse contributors the
                // server would have allowed.
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

    /// The public surface: an opt-in row off the roster, the community-brand
    /// panel on it.
    ///
    /// Two surfaces rather than two states of one. Per §7.3 the black frame
    /// is the exact boundary of what becomes public, so the change of visual
    /// language is the statement and the two are built separately.
    ///
    /// Filled from `get_public_profile`, which reports the daemon's local
    /// cache of the last claim this device made. There is no
    /// `GET /v1/community/profile` for a contributor's own row, so a cache is
    /// what any shell has: it says what this machine last published, not what
    /// the roster holds this second.
    private var publicProfile: some View {
        VStack(alignment: .leading, spacing: TC.Space.sm) {
            if let profile = model.publicProfile, let handle = profile.handle {
                profilePanel(profile, handle: handle)
            } else {
                TCSectionHeader(title: PublicProfileCopy.heading)
                optInRow
            }
            if let sentence = profileOutcomeSentence {
                Text(sentence)
                    .font(TC.Font_.body)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Text(PublicProfileCopy.footnote)
                .font(TC.Font_.caption)
                .lineSpacing(TC.Font_.LineHeight.spacing(for: 11, TC.Font_.LineHeight.caption))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            profileCopyDefects
        }
        // Seeded from the daemon's answer whenever it changes, so the fields
        // show what is actually published -- including the trimmed display
        // form the server stored, which need not be the string that was
        // typed. Keyed on the published values rather than on every render,
        // so a refresh cannot overwrite an edit in progress.
        .onAppear { seedProfileDraft() }
        .onChange(of: publishedSignature) { _, _ in seedProfileDraft() }
    }

    /// The public-profile copy's own assertions, rendered where a
    /// contributor and a developer both see them -- the same arrangement
    /// `HistoryView` uses for the withdrawal wording, and for the same
    /// reason: there is no Swift test target here, so an assertion that is
    /// not rendered is an assertion nobody runs. Empty in every healthy
    /// build.
    @ViewBuilder
    private var profileCopyDefects: some View {
        let problems = PublicProfileCopyCheck.failures()
        if !problems.isEmpty {
            VStack(alignment: .leading, spacing: TC.Space.xs) {
                Text("Do not trust the public-profile wording on this screen.")
                    .font(TC.Font_.cardTitle)
                ForEach(problems, id: \.self) { problem in
                    Text(problem).font(TC.Font_.footnote)
                }
            }
            .foregroundStyle(TC.coralText)
            .padding(TC.Space.m)
            .frame(maxWidth: .infinity, alignment: .leading)
            .tcCard()
        }
    }

    /// Off the roster. The row §5.4 names, and a button that opens the
    /// consent dialog rather than doing anything itself: going public is a
    /// consent dialog, not a toggle flip (§5.7), and the foreign visual
    /// language starts at the sheet's edge.
    private var optInRow: some View {
        HStack(alignment: .center, spacing: TC.Space.m) {
            Text(PublicProfileCopy.listHandlePublicly)
                .font(TC.Font_.body)
            Spacer(minLength: 0)
            Button(PublicProfileCopy.goPublicConfirm) { showingGoPublic = true }
                .buttonStyle(.bordered)
                .font(TC.Font_.labelControl)
        }
        .padding(TC.Space.m)
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard()
    }

    /// On the roster: §5.6's brand panel, editable.
    private func profilePanel(
        _ profile: DaemonClient.PublicProfile,
        handle: String
    ) -> some View {
        communityBrandPanel {
            HStack(alignment: .top, spacing: TC.Space.m) {
                Text(PublicProfileCopy.heading.uppercased())
                    .font(CommunityBrand.Font_.displayPanel)
                    .tracking(CommunityBrand.Font_.displayPanelTracking)
                    .foregroundStyle(CommunityBrand.ink)
                Spacer(minLength: 0)
                if let since = profile.publicSince {
                    Text(PublicProfileCopy.onRosterSince(Self.rosterDate.string(from: since)))
                        .font(CommunityBrand.Font_.labelMono)
                        .tracking(CommunityBrand.Font_.monoTracking)
                        .foregroundStyle(CommunityBrand.muted)
                        .multilineTextAlignment(.trailing)
                }
            }

            profileEditor(label: PublicProfileCopy.handleLabel, text: $handleDraft, mono: true)

            VStack(alignment: .leading, spacing: TC.Space.xs) {
                profileBioEditor(label: PublicProfileCopy.bioLabel, text: $bioDraft)
                // Counted off the value above, not the mockup's "74/280": a
                // counter that does not count is worse than no counter.
                // Bytes, because the limit is stated in bytes.
                Text("\(bioDraft.utf8.count)/280")
                    .font(CommunityBrand.Font_.labelMono)
                    .tracking(CommunityBrand.Font_.monoTracking)
                    .foregroundStyle(CommunityBrand.muted)
                    .frame(maxWidth: .infinity, alignment: .trailing)
            }

            HStack(spacing: TC.Space.sm) {
                // Save re-publishes the whole profile, because that is what
                // the PUT does: the handle and the bio as they stand, both
                // of them, every time. There is no partial update to offer.
                Button(PublicProfileCopy.saveProfile) {
                    model.claimHandle(handleDraft, bio: bioDraft)
                }
                .buttonStyle(CommunityBrandButtonStyle(fill: CommunityBrand.accent))
                .disabled(model.profileBusy || handleDraft.trimmingCharacters(
                    in: .whitespacesAndNewlines
                ).isEmpty)
                Button(PublicProfileCopy.leaveRoster) { model.leaveRoster() }
                    .buttonStyle(CommunityBrandButtonStyle(fill: CommunityBrand.paper))
                    .disabled(model.profileBusy)
            }
        }
        // The handle is what is published; the panel says so to VoiceOver
        // rather than leaving the fields to be read as unlabelled boxes.
        .accessibilityLabel("\(PublicProfileCopy.heading): \(handle)")
    }

    /// What the last claim or withdrawal did, in words.
    ///
    /// `published(cached: false)` is a **success**: the server has taken the
    /// handle, and only this device's copy of it is missing. It gets the
    /// sentence that says so rather than a refusal sentence -- a shell that
    /// reported it as a failure would tell a contributor their handle is
    /// private when it is public.
    private var profileOutcomeSentence: String? {
        switch model.profileOutcome {
        case .none: return nil
        case .published(let cached):
            return cached ? PublicProfileCopy.published : PublicProfileCopy.publishedNotCached
        case .left(let cached):
            return cached ? PublicProfileCopy.leftRoster : PublicProfileCopy.leftRosterNotCached
        case .refused(let label):
            return PublicProfileCopy.failureSentence(label)
        case .leaveRefused(let label):
            return PublicProfileCopy.leaveFailureSentence(label)
        }
    }

    /// The published values, as one string, so the drafts are re-seeded when
    /// and only when the daemon's answer actually changes.
    private var publishedSignature: String {
        "\(model.publicProfile?.handle ?? "")\u{1}\(model.publicProfile?.bio ?? "")"
    }

    private func seedProfileDraft() {
        handleDraft = model.publicProfile?.handle ?? ""
        bioDraft = model.publicProfile?.bio ?? ""
    }

    private static let rosterDate: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateStyle = .long
        formatter.timeStyle = .none
        return formatter
    }()

    /// Spec §6.10: a brand field box is `border:1px solid #000`,
    /// `padding:8px 12px`, no radius, with its `label.mono` above it.
    private func profileEditor(
        label: String,
        text: Binding<String>,
        mono: Bool
    ) -> some View {
        VStack(alignment: .leading, spacing: TC.Space.xs) {
            brandFieldLabel(label)
            TextField("", text: text)
                .textFieldStyle(.plain)
                .font(mono ? CommunityBrand.Font_.fieldValueMono : CommunityBrand.Font_.fieldValue)
                .tracking(CommunityBrand.Font_.fieldValueTracking)
                .foregroundStyle(CommunityBrand.ink)
                .padding(.vertical, TC.Space.s)
                .padding(.horizontal, TC.Space.m)
                .overlay(
                    Rectangle().strokeBorder(
                        CommunityBrand.ink,
                        lineWidth: CommunityBrand.Metric.rule
                    )
                )
                .accessibilityLabel(label)
        }
    }

    private func profileBioEditor(label: String, text: Binding<String>) -> some View {
        VStack(alignment: .leading, spacing: TC.Space.xs) {
            brandFieldLabel(label)
            TextEditor(text: text)
                .font(CommunityBrand.Font_.fieldValue)
                .foregroundStyle(CommunityBrand.ink)
                // The editor paints its own ground, which would be the
                // system's rather than the brand's paper inside a black
                // frame.
                .scrollContentBackground(.hidden)
                .background(CommunityBrand.paper)
                .frame(minHeight: 56)
                .padding(.vertical, TC.Space.s)
                .padding(.horizontal, TC.Space.m)
                .overlay(
                    Rectangle().strokeBorder(
                        CommunityBrand.ink,
                        lineWidth: CommunityBrand.Metric.rule
                    )
                )
                .accessibilityLabel(label)
        }
    }

    private func brandFieldLabel(_ text: String) -> some View {
        Text(text.uppercased())
            .font(CommunityBrand.Font_.labelMono)
            .tracking(CommunityBrand.Font_.monoTracking)
            .foregroundStyle(CommunityBrand.muted)
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

    // MARK: - The local change log

    /// What has been changed on this machine, from the daemon's `list_audit`.
    ///
    /// The shared design spec does not draw this surface -- the Linux shell
    /// is the only prior art -- so the section heading, the empty sentence
    /// and every action sentence below are the Linux shell's own words
    /// (`crates/trace-commons-contributor-gtk/src/ui/settings.rs`) rather
    /// than new copy. Two shells narrating the same log differently is a
    /// worse outcome than either wording on its own.
    ///
    /// Every value drawn here is a fixed label by contract: the instant, the
    /// action name mapped to a sentence, and the daemon-derived project
    /// label. Nothing on this screen may be enriched with a path, a token, a
    /// tenant, a session hash or a trace body -- see `AuditEntry`. And per
    /// the contract this is a record, not a guard: nothing in this app
    /// decides anything on the strength of what is listed here.
    private var audit: some View {
        VStack(alignment: .leading, spacing: TC.Space.sm) {
            TCSectionHeader(title: "What has been changed on this machine")
            if model.audit.isEmpty {
                Text("Nothing has been changed.")
                    .font(TC.Font_.meta)
                    .foregroundStyle(.secondary)
            }
            // The rows carry no id on the wire, and two entries can legally
            // agree on every field they do carry (the same action, on the
            // same project, within the same second), so the offset in a
            // newest-first list is the only stable identity available.
            ForEach(Array(model.audit.enumerated()), id: \.offset) { _, entry in
                auditRow(entry)
            }
        }
    }

    private func auditRow(_ entry: AuditEntry) -> some View {
        // The instant is a figure, so it is set as one, and the column of
        // them lines up down the section -- same reasoning as the Linux
        // shell's ledger treatment.
        HStack(alignment: .firstTextBaseline, spacing: TC.Space.m) {
            Text(Self.instant(entry.at))
                .font(TC.Font_.ledger)
                .foregroundStyle(.secondary)
            Text(Self.auditSentence(entry.action, project: entry.projectLabel))
                .font(TC.Font_.meta)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
        .accessibilityElement(children: .combine)
    }

    /// The Linux shell prints the raw RFC 3339 instant because GTK has it as
    /// a string; this layer has already decoded it to a `Date`, so it is
    /// shown in the reader's own locale and time zone. Same fact, stated the
    /// way the platform states dates elsewhere in this app.
    private static func instant(_ date: Date) -> String {
        date.formatted(.dateTime.month(.abbreviated).day().hour().minute())
    }

    /// Fixed action labels to sentences. The wording is the Linux shell's
    /// `audit_sentence`, verbatim, including its catch-all: an action this
    /// build does not know still gets a row, because a change that happened
    /// and is not listed is exactly what this log exists to prevent.
    private static func auditSentence(_ action: String, project: String?) -> String {
        let sentence: String
        switch action {
        case "armed-auto-upload": sentence = "Automatic contributing turned on for"
        case "disarmed-auto-upload": sentence = "Automatic contributing turned off for"
        case "queue-bulk-approved": sentence = "The whole queue was approved"
        case "consent-scopes-changed": sentence = "Permissions changed"
        case "near-ai-notice-acknowledged": sentence = "The extra privacy scan was confirmed"
        default: sentence = "Changed"
        }
        guard let project, !project.isEmpty else { return sentence }
        return "\(sentence) \(project)"
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

    @EnvironmentObject private var model: AppModel
    @State private var acknowledged = false
    @State private var handle = ""
    @State private var bio = ""

    /// Spec §4.6: the dialog is drawn at 560px.
    private static let width: CGFloat = 560

    /// The acknowledgement gate, plus the one thing the call cannot be made
    /// without. Both are the same rule stated twice: the primary does
    /// nothing until there is something to consent to and a consent to it.
    private var canGoPublic: Bool {
        acknowledged
            && !handle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !model.profileBusy
    }

    /// A refusal stays in the dialog, next to the field it is about: the one
    /// thing wanted after "that handle is reserved" is the box it was typed
    /// into. A success closes the sheet, and the Settings panel behind it
    /// reports what was published.
    private var refusal: String? {
        if case .refused(let label) = model.profileOutcome {
            return PublicProfileCopy.failureSentence(label)
        }
        return nil
    }

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.l) {
            Text(PublicProfileCopy.goPublicHeadline.uppercased())
                .font(CommunityBrand.Font_.displayDialog)
                .tracking(CommunityBrand.Font_.displayDialogTracking)
                .foregroundStyle(CommunityBrand.ink)
                .fixedSize(horizontal: false, vertical: true)

            consentColumns

            // The handle itself, inside the consent dialog rather than
            // behind it: the thing being consented to is this exact string
            // becoming public, and nobody can meaningfully acknowledge "my
            // handle becomes public" and then be asked afterwards what the
            // handle is.
            fields

            acknowledgement

            if let refusal {
                Text(refusal)
                    .font(CommunityBrand.Font_.body)
                    .tracking(CommunityBrand.Font_.bodyTracking)
                    .foregroundStyle(CommunityBrand.ink)
                    .fixedSize(horizontal: false, vertical: true)
            }

            HStack(spacing: TC.Space.sm) {
                Spacer(minLength: 0)
                Button(PublicProfileCopy.notNow) { onDismiss() }
                    .buttonStyle(CommunityBrandButtonStyle(fill: CommunityBrand.paper))
                Button(PublicProfileCopy.goPublicConfirm) {
                    model.claimHandle(handle, bio: bio)
                }
                .buttonStyle(CommunityBrandButtonStyle(fill: CommunityBrand.accent))
                .disabled(!canGoPublic)
            }

            Text(PublicProfileCopy.goPublicFootnote)
                .font(CommunityBrand.Font_.footnote)
                .foregroundStyle(CommunityBrand.muted)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(TC.Space.xl)
        .frame(width: Self.width, alignment: .leading)
        .background(CommunityBrand.paper)
        // Any outcome that is not a refusal is a claim the server accepted,
        // including one this device failed to cache: the handle is on the
        // roster either way, so the dialog's work is done and the sentence
        // for it belongs on the panel behind, not here.
        .onChange(of: outcomeIsSettled) { _, settled in
            if settled { onDismiss() }
        }
        // A stale refusal from an earlier attempt must not greet the next
        // opening of this sheet.
        .onAppear { model.clearProfileOutcome() }
    }

    private var outcomeIsSettled: Bool {
        switch model.profileOutcome {
        case .published, .left: return true
        case .none, .refused, .leaveRefused: return false
        }
    }

    /// Spec §6.10's brand field boxes, empty and waiting.
    private var fields: some View {
        VStack(alignment: .leading, spacing: TC.Space.m) {
            VStack(alignment: .leading, spacing: TC.Space.xs) {
                fieldLabel(PublicProfileCopy.goPublicHandleLabel)
                TextField("", text: $handle)
                    .textFieldStyle(.plain)
                    .font(CommunityBrand.Font_.fieldValueMono)
                    .tracking(CommunityBrand.Font_.fieldValueTracking)
                    .foregroundStyle(CommunityBrand.ink)
                    .padding(.vertical, TC.Space.s)
                    .padding(.horizontal, TC.Space.m)
                    .overlay(
                        Rectangle().strokeBorder(
                            CommunityBrand.ink,
                            lineWidth: CommunityBrand.Metric.rule
                        )
                    )
                    .accessibilityLabel(PublicProfileCopy.goPublicHandleLabel)
            }
            VStack(alignment: .leading, spacing: TC.Space.xs) {
                fieldLabel(PublicProfileCopy.goPublicBioLabel)
                TextEditor(text: $bio)
                    .font(CommunityBrand.Font_.fieldValue)
                    .foregroundStyle(CommunityBrand.ink)
                    .scrollContentBackground(.hidden)
                    .background(CommunityBrand.paper)
                    .frame(minHeight: 56)
                    .padding(.vertical, TC.Space.s)
                    .padding(.horizontal, TC.Space.m)
                    .overlay(
                        Rectangle().strokeBorder(
                            CommunityBrand.ink,
                            lineWidth: CommunityBrand.Metric.rule
                        )
                    )
                    .accessibilityLabel(PublicProfileCopy.goPublicBioLabel)
                // Bytes, because the limit is stated in bytes.
                Text("\(bio.utf8.count)/280")
                    .font(CommunityBrand.Font_.labelMono)
                    .tracking(CommunityBrand.Font_.monoTracking)
                    .foregroundStyle(CommunityBrand.muted)
                    .frame(maxWidth: .infinity, alignment: .trailing)
            }
        }
    }

    private func fieldLabel(_ text: String) -> some View {
        Text(text.uppercased())
            .font(CommunityBrand.Font_.labelMono)
            .tracking(CommunityBrand.Font_.monoTracking)
            .foregroundStyle(CommunityBrand.muted)
    }

    /// A single 2px box split by one 1px rule, per §5.7. The two columns are
    /// deliberately the same weight: what is published and what never is are
    /// the same size of fact.
    private var consentColumns: some View {
        HStack(alignment: .top, spacing: 0) {
            column(
                title: PublicProfileCopy.publishedHeading,
                lines: [
                    "Your handle — real handles only, no pseudonyms.",
                    "Aggregate counts: accepted, novelty credit, accept rate.",
                    "The date you went public.",
                    "Your bio, if you write one."
                ]
            )
            Rectangle().fill(CommunityBrand.ink).frame(width: CommunityBrand.Metric.rule)
            column(
                title: PublicProfileCopy.neverHeading,
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
                Text(PublicProfileCopy.goPublicAcknowledgement)
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

