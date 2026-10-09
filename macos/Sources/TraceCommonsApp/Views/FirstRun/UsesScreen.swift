import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The Uses screen's decisions, apart from the view so they can be tested.
/// Every string is the core's; these only choose and fill placeholders.
enum UsesScreenLayout {
    /// The optional group's checkbox: every optional use on, some, or none.
    enum Group: Equatable {
        case all
        case some
        case none
    }

    /// The floor scope (`consent_options`' `always_on`), shown ticked,
    /// locked and required: it is always included (owner, 2026-10-08).
    static func requiredScope(_ options: [ConsentScope]) -> ConsentScope? {
        options.first(where: \.alwaysOn)
    }

    /// The other data uses, inside the expander.
    static func optionalScopes(_ options: [ConsentScope]) -> [ConsentScope] {
        options.filter { !$0.alwaysOn && $0.grantsDataUse }
    }

    /// Scopes that grant no data use (the public handle), beside them.
    static func handleScopes(_ options: [ConsentScope]) -> [ConsentScope] {
        options.filter { !$0.alwaysOn && !$0.grantsDataUse }
    }

    /// What the optional group's expander opens onto: the optional data
    /// uses only. The handle is a row of its own after the group (Ron's
    /// review of #1235, item 2).
    static func expandedScopes(_ options: [ConsentScope]) -> [ConsentScope] {
        optionalScopes(options)
    }

    /// The scope rows on screen, in order: the required use, the optional
    /// uses while the group is open, then the handle, always.
    static func visibleScopes(_ options: [ConsentScope], optionalOpen: Bool) -> [ConsentScope] {
        (requiredScope(options).map { [$0] } ?? []) + (optionalOpen ? expandedScopes(options) : [])
            + handleScopes(options)
    }

    static func isTicked(_ scope: String, in state: FirstRunState) -> Bool {
        state.scopes.contains(scope)
    }

    static func group(_ scopes: Set<String>, optional: [ConsentScope]) -> Group {
        let on = optional.filter { scopes.contains($0.name) }.count
        if !optional.isEmpty, on == optional.count { return .all }
        return on == 0 ? .none : .some
    }

    /// Ron's summary beside the expander, every placeholder filled.
    static func optionalSummary(_ uses: FirstRunCopy.Uses, scopes: Set<String>, optional: [ConsentScope]) -> String {
        let on = optional.filter { scopes.contains($0.name) }.count
        let template: String
        switch group(scopes, optional: optional) {
        case .all: template = uses.optionalAllOn
        case .none: template = uses.optionalAllOff
        case .some: template = uses.optionalSomeOn
        }
        return FirstRunCopy.fill(template, ["count": String(optional.count), "selected": String(on)])
    }

    /// Start: the required use known and an account Start can finish
    /// (`FirstRunNavigation.canContinue`), the sharing line for the path
    /// shown present (the fallback says Starting is disabled, so it is),
    /// and no Start already running. The required use is not waited on:
    /// it is always included (`includingRequired`).
    static func canStart(
        _ state: FirstRunState, uses: FirstRunCopy.Uses, requiredScope: ConsentScope?, grant: AutomaticGrantCopy?,
        isCommitting: Bool
    ) -> Bool {
        guard !isCommitting,
            sharingLine(uses, path: effectiveSharing(state), grant: grant) != uses.sharingUnavailable
        else { return false }
        return FirstRunNavigation.canContinue(state, candidates: [], requiredScope: requiredScope?.name)
    }

    /// Where Start goes.
    enum StartRoute: Equatable {
        /// The two disclosures, then the core's answer, then the commit.
        case disclose
        /// Private AI on without Automatic: the witness disclosure, then
        /// the commit, which asks for no grant.
        case discloseWitness
        /// Straight to the commit.
        case commit
    }

    /// Automatic, on an account that can choose it, goes through both
    /// disclosures; Private AI turned on otherwise goes through the witness
    /// disclosure (spec rule 10); everything else commits directly.
    static func startRoute(_ state: FirstRunState) -> StartRoute {
        if SharingDisclosureFlow.isNeeded(for: state) { return .disclose }
        if SharingDisclosureFlow.isWitnessOnlyNeeded(for: state) { return .discloseWitness }
        return .commit
    }

    /// The state Start commits: the required use always among its scopes,
    /// as its locked, ticked box shows (`FirstRunState.includingRequiredScope`).
    static func includingRequired(_ state: FirstRunState, required: ConsentScope?) -> FirstRunState {
        state.includingRequiredScope(required?.name)
    }

    /// The path the screen shows: Automatic only for an account that can
    /// choose it, so a watch-only state never reads Automatic's words.
    static func effectiveSharing(_ state: FirstRunState) -> SharingPath {
        FirstRunNavigation.canChooseAutomatic(state) ? state.sharing : .askMe
    }

    /// The Sharing card's line: the first line of the core's words for the
    /// path chosen (owner, 2026-10-08), the loading line until they are
    /// read, or the fallback that disables Start. The rest is
    /// `sharingDetail`, behind an info button; a copy missing either half
    /// reads the fallback, so nothing of the answer is shown without the
    /// rest being there to read.
    static func sharingLine(
        _ uses: FirstRunCopy.Uses, path: SharingPath, grant: AutomaticGrantCopy?, isLoading: Bool = false
    ) -> String {
        if isLoading { return uses.sharingLoading }
        guard let grant, let split = split(path: path, grant: grant) else { return uses.sharingUnavailable }
        return split.line
    }

    /// The rest of the path's words, behind the Sharing line's info button:
    /// Ask me's detail; Automatic's detail, then the scrub's scope and
    /// limit. Nil whenever the line is a fallback.
    static func sharingDetail(path: SharingPath, grant: AutomaticGrantCopy?) -> String? {
        grant.flatMap { split(path: path, grant: $0)?.detail }
    }

    private static func split(path: SharingPath, grant: AutomaticGrantCopy) -> (line: String, detail: String)? {
        switch path {
        case .askMe:
            guard let line = grant.pathAskFirstTitle, !line.isEmpty, let detail = grant.pathAskFirstDetail,
                !detail.isEmpty
            else { return nil }
            return (line, detail)
        case .automatic:
            guard let line = grant.pathAutomaticTitle, !line.isEmpty, let detail = grant.pathAutomaticDetail,
                !detail.isEmpty, let scrub = grant.scrub
            else { return nil }
            return (line, [detail, scrub.scope, scrub.limit].joined(separator: " "))
        }
    }

    /// The info button's accessible name for a row titled `title`.
    static func moreAbout(_ title: String, uses: FirstRunCopy.Uses) -> String {
        FirstRunCopy.fill(uses.moreAbout, ["title": title])
    }

    /// The picker's options, named by the contribution mode table (Ask me,
    /// Automatic), and Automatic only for an account that can choose it.
    static func sharingOptions(
        for state: FirstRunState, modes: ContributionModeCopy?
    ) -> [GlassPickerOption<SharingPath>] {
        FirstRunNavigation.sharingPaths(for: state).compactMap { path in
            let mode: ProjectMode = path == .automatic ? .autoUpload : .ask
            guard let title = modes?.label(for: mode) else { return nil }
            return GlassPickerOption(title, value: path, dot: path == .automatic ? .on : .ask)
        }
    }

    /// The Private AI card is Custom's (Ron's W-6); Quick has none.
    static func showsPrivateAI(_ state: FirstRunState) -> Bool {
        state.tier == .custom
    }

    /// The notice for the runner's failure on this screen, one for each
    /// way Start can end. A refused grant comes after setup finished, so it
    /// reads the first run's own line whatever the label; the Private AI
    /// failure reads the Private AI copy's, or the first run's without it.
    static func notice(
        for failure: FirstRunFailure?, uses: FirstRunCopy.Uses, privateAI: PrivateInferenceCopy?
    ) -> String? {
        switch failure {
        case .none: return nil
        case .grantRefused: return uses.sharingRefused
        case .scopesFailed: return uses.scopesFailed
        case .rulesFailed: return uses.rulesFailed
        case .privateAIFailed: return privateAI?.writeUnconfirmed ?? uses.privateAiFailed
        // The marker was not written, so setup did not finish: its own
        // line, never the refused grant's "Setup finished".
        case .completeFailed: return uses.completeFailed
        // Leaving the roots' failures, which never stop Start.
        case .startFailed, .passkeyUnavailable, .settingsFailed, .inviteDead, .lookupUnavailable, .enrollFailed, .signInFailed,
            .nearAIEnrollFailed:
            return nil
        }
    }

    /// What a finished first run must still say once the first-run host has
    /// gone: Automatic was refused, so sharing is on Ask me. Shown by the
    /// shell notices (`AppModel.firstRunNotice`), not by this screen.
    static func finishedNotice(_ refusal: FirstRunFailure?, uses: FirstRunCopy.Uses) -> String? {
        if case .grantRefused? = refusal { return uses.sharingRefused }
        return nil
    }

    /// The refusal a Start carries. A fresh pass through the disclosures
    /// supersedes any earlier verdict; a plain Start keeps the earlier one.
    static func refusalToCarry(
        decidedNow: Bool, refused: FirstRunFailure?, pending: FirstRunFailure?
    ) -> FirstRunFailure? {
        decidedNow ? refused : pending
    }

    /// After a Start: the failure to show, and a refusal decided before it
    /// that is still to be shown. A Start that failed keeps the refusal for
    /// the retry, which goes through on Ask me; one that succeeded shows it.
    static func afterStart(
        failure: FirstRunFailure?, pending: FirstRunFailure?
    ) -> (shown: FirstRunFailure?, pending: FirstRunFailure?) {
        guard failure == nil else { return (failure, pending) }
        return (pending, nil)
    }
}

/// Start's two commits, apart from the view so a runner can drive them.
/// Each returns the refusal still to be shown after a later Start.
@MainActor
enum UsesStart {
    /// Start without the disclosures (Ask me, watching only). It carries an
    /// earlier refusal; it cannot grant, since only `finish` marks the
    /// grant ready.
    static func plainStart(runner: FirstRunRunner, pending: FirstRunFailure?) async -> FirstRunFailure? {
        let carried = UsesScreenLayout.refusalToCarry(decidedNow: false, refused: nil, pending: pending)
        return await commit(runner: runner, carrying: carried)
    }

    /// Both disclosures seen: the core's answer decides the state Start
    /// commits (Automatic with `grantReady`, or Ask me with the refusal),
    /// then Start runs.
    static func finish(
        runner: FirstRunRunner, request: Flow1GrantRequest?, pending: FirstRunFailure?
    ) async -> FirstRunFailure? {
        let (next, refused) = SharingDisclosureFlow.resolve(runner.state, request: request)
        runner.state = next
        let carried = UsesScreenLayout.refusalToCarry(decidedNow: true, refused: refused, pending: pending)
        return await commit(runner: runner, carrying: carried)
    }

    /// The one Start path. A refusal is kept past a failed Start and shown
    /// once one succeeds; the commit clears failures when it begins.
    private static func commit(runner: FirstRunRunner, carrying refusal: FirstRunFailure?) async -> FirstRunFailure? {
        // The refusal goes into the commit too: once the marker is written
        // the host can leave before this function resumes.
        await runner.commit(.start, carrying: refusal)
        // `completeFailed` outranks a refusal the daemon gave in this same
        // Start; that refusal is kept for the retry, like one decided before.
        let kept = runner.failure == .completeFailed ? runner.refusedGrant ?? refusal : refusal
        let after = UsesScreenLayout.afterStart(failure: runner.failure, pending: kept)
        runner.failure = after.shown
        return after.pending
    }
}

/// Ron's Uses screen (#1030 `uses-screen.tsx`, W-3 and W-6) in glass: how
/// traces may be used, the Sharing decision, the Private AI switch on
/// Custom, and Start. Automatic leads through `SharingDisclosureSheet`
/// before the start commit.
struct UsesScreen: View {
    @EnvironmentObject private var model: AppModel
    let copy: FirstRunCopy
    @ObservedObject var runner: FirstRunRunner

    @State private var optionalOpen = false
    /// The Private AI card's "Learn more" was pressed.
    @State private var privateAIMore = false
    @State private var disclosure: SharingDisclosureFlow?
    /// The core's sharing words for this configuration, read once on
    /// appear; nil disables Start. Start writes the configuration they
    /// depend on, so they cannot change while this screen is up.
    @State private var grant: AutomaticGrantCopy?
    @State private var grantRead = false
    /// A refusal decided before a Start that then failed, shown after the
    /// Start that succeeds.
    @State private var pendingRefusal: FirstRunFailure?

    private var privateAI: PrivateInferenceCopy? { model.privateInferenceCopy }

    var body: some View {
        let options = model.consentScopes
        let required = UsesScreenLayout.requiredScope(options)
        FirstRunFrame(
            copy: copy,
            state: $runner.state,
            notice: UsesScreenLayout.notice(for: runner.failure, uses: copy.uses, privateAI: privateAI),
            footer: FirstRunFooter(
                title: copy.uses.start,
                isEnabled: UsesScreenLayout.canStart(
                    runner.state, uses: copy.uses, requiredScope: required, grant: grant,
                    isCommitting: runner.isCommitting),
                busy: runner.isCommitting,
                action: start)
        ) {
            FirstRunTitle(light: copy.uses.titleLight, bold: copy.uses.titleBold)
        } content: {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                usesCard(options: options, required: required)
                sharingCard(grant: grant)
                if UsesScreenLayout.showsPrivateAI(runner.state) {
                    privateAICard
                }
            }
        }
        // The required use is in the state as soon as it is known, so the
        // state matches its ticked box; Start includes it again regardless.
        .onChange(of: required?.name, initial: true) { _, _ in includeRequired() }
        .task {
            // `enroll` re-reads the route, not status; the grant's
            // `connected` is read from status, so it is refreshed here.
            model.refreshStatus()
            model.refreshConsentOptions()
            model.refreshWitness()
            if !grantRead {
                grant = AutomaticGrantCopy.decode(
                    fromJSON: TCCoreCopy.automaticContributionCopyJSON(configDir: model.configDirectory))
                grantRead = true
            }
        }
        .glassModal(isPresented: Binding(get: { disclosure != nil }, set: { if !$0 { disclosure = nil } })) {
            if let grant {
                SharingDisclosureSheet(copy: copy, grant: grant, flow: $disclosure, onFinish: finish)
                    .environmentObject(model)
            }
        }
    }

    // MARK: - Uses

    private func usesCard(options: [ConsentScope], required: ConsentScope?) -> some View {
        let optional = UsesScreenLayout.optionalScopes(options)
        return GlassEyebrowCard(copy.uses.eyebrow) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                if let required {
                    HStack(spacing: GlassTokens.Space.s4) {
                        // Always included, so ticked and locked: the person
                        // cannot untick it (owner, 2026-10-08).
                        Toggle(required.title, isOn: .constant(true))
                            .toggleStyle(GlassCheckboxStyle())
                            .disabled(true)
                        infoButton(title: required.title, text: required.description)
                        // Ron's inline "required" in the on colour.
                        Text(copy.uses.required)
                            .glassType(GlassTokens.TypeScale.mono)
                            .foregroundStyle(GlassTokens.Color.statusOnText.color)
                    }
                }
                if !optional.isEmpty {
                    HStack(spacing: GlassTokens.Space.s4) {
                        // #1030: "All optional uses" is the box's name, not
                        // a visible label; the expander says the rest.
                        Toggle(sources: optional.map { scope($0.name) }, isOn: \.self) {
                            Text(copy.uses.allOptional)
                                .frame(width: 0, height: 0)
                                .clipped()
                        }
                        .toggleStyle(GlassCheckboxStyle())
                        .accessibilityLabel(copy.uses.allOptional)
                        GlassExpander(
                            UsesScreenLayout.optionalSummary(copy.uses, scopes: runner.state.scopes, optional: optional),
                            isOpen: $optionalOpen)
                    }
                }
                if optionalOpen {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                        ForEach(UsesScreenLayout.expandedScopes(options)) { option in
                            scopeRow(option, options: options)
                        }
                    }
                    .padding(.leading, GlassTokens.Space.s8)
                }
                // The handle: its own row after the group, always shown.
                ForEach(UsesScreenLayout.handleScopes(options)) { option in
                    scopeRow(option, options: options)
                }
            }
        }
    }

    /// A scope's box, then its description behind an info button right
    /// after the title (owner, 2026-10-08). The description is the core
    /// consent table's.
    private func scopeRow(_ option: ConsentScope, options: [ConsentScope]) -> some View {
        HStack(spacing: GlassTokens.Space.s4) {
            Toggle(option.title, isOn: scope(option.name))
                .toggleStyle(GlassCheckboxStyle())
            infoButton(title: option.title, text: option.description)
        }
    }

    private func infoButton(title: String, text: String) -> some View {
        GlassInfoButton(UsesScreenLayout.moreAbout(title, uses: copy.uses), text: text)
    }

    /// One optional scope's box. The person's toggle is the only thing that
    /// ticks an optional scope on this screen; the required one is always
    /// included (`includeRequired`).
    private func scope(_ name: String) -> Binding<Bool> {
        Binding(
            get: { runner.state.scopes.contains(name) },
            set: { on in
                if on {
                    runner.state.scopes.insert(name)
                } else {
                    runner.state.scopes.remove(name)
                }
            })
    }

    // MARK: - Sharing

    private func sharingCard(grant: AutomaticGrantCopy?) -> some View {
        GlassCard {
            HStack(alignment: .top, spacing: GlassTokens.Space.s6) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                    Text(copy.uses.sharing)
                        .glassType(GlassTokens.TypeScale.bodyStrong)
                        .foregroundStyle(GlassColor.textPrimary)
                    // The path's first line; the rest behind the info button.
                    HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s3) {
                        cardBody(
                            UsesScreenLayout.sharingLine(
                                copy.uses, path: UsesScreenLayout.effectiveSharing(runner.state), grant: grant,
                                isLoading: !grantRead))
                        if let detail = UsesScreenLayout.sharingDetail(
                            path: UsesScreenLayout.effectiveSharing(runner.state), grant: grant)
                        {
                            infoButton(title: copy.uses.sharing, text: detail)
                        }
                    }
                }
                Spacer(minLength: 0)
                GlassPicker(
                    copy.uses.sharing,
                    selection: Binding(
                        get: { UsesScreenLayout.effectiveSharing(runner.state) },
                        set: { if let path = $0 { runner.state.sharing = path } }),
                    options: UsesScreenLayout.sharingOptions(for: runner.state, modes: ProjectModeWords.table),
                    placeholder: copy.frame.choose)
                .disabled(grant == nil)
            }
        }
    }

    // MARK: - Private AI

    /// Worded by the Private AI copy; the switch stays off and disabled
    /// until that copy is there, so nobody turns it on unread.
    private var privateAICard: some View {
        GlassCard {
            HStack(alignment: .top, spacing: GlassTokens.Space.s6) {
                // #1030 `ftux-gap-2` between the paragraphs.
                VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                    if let privateAI {
                        Text(privateAI.destination)
                            .glassType(GlassTokens.TypeScale.bodyStrong)
                            .foregroundStyle(GlassColor.textPrimary)
                        // The offer's first two sentences; "Learn more"
                        // discloses the rest in place (owner, 2026-10-08).
                        cardBody(copy.privateAi.offerLead)
                        if privateAIMore {
                            cardBody(copy.privateAi.offerMore)
                            cardBody(privateAI.offerExposure)
                            cardBody(privateAI.offerNoRepoint)
                        } else {
                            Button(copy.privateAi.learnMore) { privateAIMore = true }
                                .buttonStyle(GlassButtonStyle(.link))
                        }
                    } else {
                        cardBody(copy.privateAi.unavailable)
                    }
                }
                Spacer(minLength: 0)
                Toggle(
                    privateAI?.offerTitle ?? copy.privateAi.toggleLoading,
                    isOn: Binding(
                        get: { runner.state.privateAI && privateAI != nil },
                        set: { runner.state.privateAI = $0 })
                )
                .toggleStyle(GlassToggleStyle(.settings, showsLabel: false))
                .disabled(privateAI == nil)
            }
        }
    }

    // MARK: - Start

    /// Put the required use in the state, once it is known.
    private func includeRequired() {
        let included = UsesScreenLayout.includingRequired(
            runner.state, required: UsesScreenLayout.requiredScope(model.consentScopes))
        if included != runner.state { runner.state = included }
    }

    private func start() {
        // What Start sends matches what is shown: the required use included.
        includeRequired()
        switch UsesScreenLayout.startRoute(runner.state) {
        case .disclose:
            disclosure = SharingDisclosureFlow()
        case .discloseWitness:
            disclosure = SharingDisclosureFlow(witnessOnly: true)
        case .commit:
            Task { pendingRefusal = await UsesStart.plainStart(runner: runner, pending: pendingRefusal) }
        }
    }

    /// Both disclosures seen: the core decides the grant, then Start runs.
    /// A not-ready answer finishes on Ask me; its failure is shown after
    /// the commit, which clears failures when it begins.
    private func finish(_ flow: SharingDisclosureFlow) {
        // The Private AI path saw the witness disclosure and asks for no
        // grant: Start as on Ask me.
        if flow.witnessOnly {
            Task { pendingRefusal = await UsesStart.plainStart(runner: runner, pending: pendingRefusal) }
            return
        }
        let request = SharingDisclosureFlow.grantRequest(
            flow.progress(connected: model.status.loggedIn, scopes: runner.state.scopes))
        Task { pendingRefusal = await UsesStart.finish(runner: runner, request: request, pending: pendingRefusal) }
    }

    /// The Sharing and Private AI cards' words (#1030 `tc-label` in
    /// secondary ink), not the tertiary caption.
    private func cardBody(_ text: String) -> some View {
        Text(text)
            .glassType(GlassTokens.TypeScale.label.weight(.regular))
            .foregroundStyle(GlassColor.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
    }
}
