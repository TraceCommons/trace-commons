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

    /// The floor scope (`consent_options`' `always_on`), shown unticked and
    /// required.
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
        return template
            .replacingOccurrences(of: "{count}", with: String(optional.count))
            .replacingOccurrences(of: "{selected}", with: String(on))
    }

    /// Start: the required use ticked (`FirstRunNavigation.canContinue`),
    /// the sharing words present, and no Start already running.
    static func canStart(
        _ state: FirstRunState, requiredScope: ConsentScope?, grant: AutomaticGrantCopy?, isCommitting: Bool
    ) -> Bool {
        guard grant != nil, !isCommitting else { return false }
        return FirstRunNavigation.canContinue(state, candidates: [], requiredScope: requiredScope?.name)
    }

    /// Ron's footer note, while the required use is unticked.
    static func footerNote(_ uses: FirstRunCopy.Uses, state: FirstRunState, requiredScope: ConsentScope?) -> String? {
        guard let requiredScope, state.scopes.contains(requiredScope.name) else { return uses.baseUseNote }
        return nil
    }

    /// The path the screen shows: Automatic only for an account that can
    /// choose it, so a watch-only state never reads Automatic's words.
    static func effectiveSharing(_ state: FirstRunState) -> SharingPath {
        FirstRunNavigation.canChooseAutomatic(state.account) ? state.sharing : .askMe
    }

    /// The Sharing card's line: the core's words for the path chosen, the
    /// loading line until they are read, or the fallback that disables
    /// Start.
    static func sharingLine(
        _ uses: FirstRunCopy.Uses, path: SharingPath, grant: AutomaticGrantCopy?, isLoading: Bool = false
    ) -> String {
        if isLoading { return uses.sharingLoading }
        guard let grant else { return uses.sharingUnavailable }
        switch path {
        case .askMe:
            guard let line = grant.pathAskFirst, !line.isEmpty else { return uses.sharingUnavailable }
            return line
        case .automatic:
            guard let line = grant.pathAutomatic, !line.isEmpty, let scrub = grant.scrub else {
                return uses.sharingUnavailable
            }
            return [line, scrub.scope, scrub.limit].joined(separator: " ")
        }
    }

    /// The picker's options, named by the contribution mode table (Ask me,
    /// Automatic), and Automatic only for an account that can choose it.
    static func sharingOptions(
        for account: AccountAnswer, modes: ContributionModeCopy?
    ) -> [GlassPickerOption<SharingPath>] {
        FirstRunNavigation.sharingPaths(for: account).compactMap { path in
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
        case .startFailed, .inviteDead, .enrollFailed, .signInFailed: return nil
        }
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

/// Ron's Uses screen (#1030 `uses-screen.tsx`, W-3 and W-6) in glass: how
/// traces may be used, the Sharing decision, the Private AI switch on
/// Custom, and Start. Automatic leads through `SharingDisclosureSheet`
/// before the start commit.
struct UsesScreen: View {
    @EnvironmentObject private var model: AppModel
    @ObservedObject var runner: FirstRunRunner
    let copy: FirstRunCopy

    @State private var optionalOpen = false
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
            onBack: runner.isCommitting ? nil : { runner.state = FirstRunNavigation.back(runner.state) },
            notice: UsesScreenLayout.notice(for: runner.failure, uses: copy.uses, privateAI: privateAI),
            footer: FirstRunFooter(
                title: copy.uses.start,
                isEnabled: UsesScreenLayout.canStart(
                    runner.state, requiredScope: required, grant: grant, isCommitting: runner.isCommitting),
                note: UsesScreenLayout.footerNote(copy.uses, state: runner.state, requiredScope: required),
                action: start)
        ) {
            ScrollView {
                VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                    Text("\(copy.uses.titleLight)\(Text(copy.uses.titleBold).fontWeight(.semibold))")
                        .glassType(GlassTokens.TypeScale.heading)
                        .foregroundStyle(GlassColor.textPrimary)
                    usesCard(options: options, required: required)
                    sharingCard(grant: grant)
                    if UsesScreenLayout.showsPrivateAI(runner.state) {
                        privateAICard
                    }
                }
            }
        }
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
        .sheet(isPresented: Binding(get: { disclosure != nil }, set: { if !$0 { disclosure = nil } })) {
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
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                        HStack(spacing: GlassTokens.Space.s4) {
                            Toggle(ScopeCopy.title(for: required.name, options: options), isOn: scope(required.name))
                                .toggleStyle(GlassCheckboxStyle())
                            GlassTag(copy.uses.required, tone: .on)
                        }
                        caption(required.description)
                    }
                }
                if !optional.isEmpty {
                    HStack(spacing: GlassTokens.Space.s4) {
                        Toggle(sources: optional.map { scope($0.name) }, isOn: \.self) {
                            Text(copy.uses.allOptional)
                        }
                        .toggleStyle(GlassCheckboxStyle())
                        GlassExpander(
                            UsesScreenLayout.optionalSummary(copy.uses, scopes: runner.state.scopes, optional: optional),
                            isOpen: $optionalOpen)
                    }
                }
                if optionalOpen {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                        ForEach(optional + UsesScreenLayout.handleScopes(options)) { option in
                            scopeRow(option, options: options)
                        }
                    }
                    .padding(.leading, GlassTokens.Space.s8)
                }
            }
        }
    }

    private func scopeRow(_ option: ConsentScope, options: [ConsentScope]) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
            Toggle(ScopeCopy.title(for: option.name, options: options), isOn: scope(option.name))
                .toggleStyle(GlassCheckboxStyle())
            caption(option.description)
        }
    }

    /// One scope's box. The person's toggle is the only thing that ticks a
    /// scope on this screen.
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
                    caption(
                        UsesScreenLayout.sharingLine(
                            copy.uses, path: UsesScreenLayout.effectiveSharing(runner.state), grant: grant,
                            isLoading: !grantRead))
                }
                Spacer(minLength: 0)
                GlassPicker(
                    copy.uses.sharing,
                    selection: Binding(
                        get: { UsesScreenLayout.effectiveSharing(runner.state) },
                        set: { if let path = $0 { runner.state.sharing = path } }),
                    options: UsesScreenLayout.sharingOptions(for: runner.state.account, modes: ProjectModeWords.table),
                    placeholder: copy.uses.sharing)
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
                VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                    if let privateAI {
                        Text(privateAI.destination)
                            .glassType(GlassTokens.TypeScale.bodyStrong)
                            .foregroundStyle(GlassColor.textPrimary)
                        caption(privateAI.offerWhat)
                        caption(privateAI.offerExposure)
                        caption(privateAI.offerNoRepoint)
                    } else {
                        caption(copy.privateAi.unavailable)
                    }
                }
                Spacer(minLength: 0)
                Toggle(
                    privateAI?.offerTitle ?? copy.privateAi.toggleLoading,
                    isOn: Binding(
                        get: { runner.state.privateAI && privateAI != nil },
                        set: { runner.state.privateAI = $0 })
                )
                .toggleStyle(GlassToggleStyle(.standard, showsLabel: false))
                .disabled(privateAI == nil)
            }
        }
    }

    // MARK: - Start

    private func start() {
        if SharingDisclosureFlow.isNeeded(for: runner.state) {
            disclosure = SharingDisclosureFlow()
            return
        }
        let carried = UsesScreenLayout.refusalToCarry(decidedNow: false, refused: nil, pending: pendingRefusal)
        Task { await commitStart(carrying: carried) }
    }

    /// Both disclosures seen: the core decides the grant, then Start runs.
    /// A not-ready answer finishes on Ask me; its failure is shown after
    /// the commit, which clears failures when it begins.
    private func finish(_ flow: SharingDisclosureFlow) {
        let request = SharingDisclosureFlow.grantRequest(
            flow.progress(connected: model.status.loggedIn, scopes: runner.state.scopes))
        let (next, refused) = SharingDisclosureFlow.resolve(runner.state, request: request)
        runner.state = next
        let carried = UsesScreenLayout.refusalToCarry(decidedNow: true, refused: refused, pending: pendingRefusal)
        Task { await commitStart(carrying: carried) }
    }

    /// The one Start path. A refusal is kept past a failed Start and shown
    /// once one succeeds.
    private func commitStart(carrying refusal: FirstRunFailure?) async {
        await runner.commit(.start)
        let after = UsesScreenLayout.afterStart(failure: runner.failure, pending: refusal)
        runner.failure = after.shown
        pendingRefusal = after.pending
    }

    private func caption(_ text: String) -> some View {
        Text(text)
            .glassType(GlassTokens.TypeScale.caption)
            .foregroundStyle(GlassColor.textTertiary)
            .fixedSize(horizontal: false, vertical: true)
    }
}
