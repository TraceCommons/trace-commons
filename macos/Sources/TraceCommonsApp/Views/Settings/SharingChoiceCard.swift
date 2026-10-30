import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// Settings > Projects' sharing choice: setup's Automatic / Ask each time,
/// changeable at any time (owner, 2026-10-30; #1753 A2). Pure, so the rules
/// can be tested apart from the card.
///
/// - Ask each time withdraws the Automatic grant and puts every folder it
///   armed back to Ask. A folder set to Automatic itself still sends.
/// - Automatic is the setup grant, behind setup's two disclosures. When
///   folders are set to Ask it asks first whether to keep them asking or
///   make every folder Automatic. Never always stays Never.
enum SharingChoice {
    /// What is in force: Automatic while the include-existing grant is,
    /// Ask each time otherwise. Nil until `automatic_grant` has answered,
    /// so nothing is shown picked on a guess.
    static func current(_ grant: AutomaticGrant?) -> SharingPath? {
        guard let grant else { return nil }
        return grant.granted && grant.includeExisting == true ? .automatic : .askMe
    }

    /// Folders set to Ask by a rule of their own: what Automatic keeps
    /// asking, or makes Automatic, as the person chooses.
    static func askingFolders(_ projects: [ProjectRow]) -> Int {
        projects.filter { $0.configured && !$0.isUnresolvedBucket && ($0.folderMode ?? $0.mode) == .ask }.count
    }

    /// What pressing Automatic shows first.
    enum Step: Equatable {
        /// The question about the folders set to Ask.
        case keepAsks(count: Int)
        /// Setup's disclosures, then the grant.
        case disclose(keepFolderAsks: Bool)
    }

    static func firstStep(asking: Int) -> Step {
        asking > 0 ? .keepAsks(count: asking) : .disclose(keepFolderAsks: true)
    }

    /// The radios: setup's own titles and lines (`UsesScreenLayout`), for
    /// an account that holds an enrollment, so Settings and setup say the
    /// same thing.
    static func options(
        copy: FirstRunCopy, modes: ContributionModeCopy?, grant: AutomaticGrantCopy?
    ) -> [GlassRadioOption<SharingPath?>] {
        let state = FirstRunState(account: .enrolled)
        return UsesScreenLayout.sharingOptions(for: state, modes: modes, grant: grant, uses: copy.uses)
    }
}

/// The card at the top of Settings > Projects. Every word is the core's.
struct SharingChoiceCard: View {
    @EnvironmentObject private var model: AppModel
    @State private var grantCopy: AutomaticGrantCopy?
    @State private var keepAsksCount: Int?
    @State private var keepFolderAsks = true
    @State private var disclosure: SharingDisclosureFlow?
    @State private var working = false
    @State private var refusal: String?

    private static let copy = TCCoreCopy.firstRunCopyJSON().flatMap(FirstRunCopy.decode)

    var body: some View {
        if let copy = Self.copy {
            card(copy)
                .task {
                    model.refreshRouteDisclosure()
                    if grantCopy == nil {
                        grantCopy = AutomaticGrantCopy.decode(
                            fromJSON: TCCoreCopy.automaticContributionCopyJSON(configDir: model.configDirectory))
                    }
                }
                .glassModal(isPresented: Binding(
                    get: { keepAsksCount != nil },
                    set: { if !$0 { keepAsksCount = nil } }
                )) {
                    if let count = keepAsksCount { keepAsksQuestion(copy, count: count) }
                }
                .glassModal(isPresented: Binding(
                    get: { disclosure != nil },
                    set: { if !$0 { disclosure = nil } }
                )) {
                    if let grantCopy {
                        SharingDisclosureSheet(copy: copy, grant: grantCopy, flow: $disclosure, onFinish: finish)
                            .environmentObject(model)
                    }
                }
        }
    }

    private func card(_ copy: FirstRunCopy) -> some View {
        GlassEyebrowCard(copy.uses.sharing) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                GlassRadioGroup(
                    copy.uses.sharing,
                    selection: Binding(
                        get: { SharingChoice.current(model.automaticGrant) },
                        set: { choose($0) }),
                    options: SharingChoice.options(copy: copy, modes: ProjectModeWords.table, grant: grantCopy))
                    .disabled(working || model.automaticGrant == nil || grantCopy == nil)
                if let note = copy.uses.settingsSharingNote {
                    Text(note)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if let refusal {
                    GlassAlert(refusal)
                }
            }
        }
    }

    private func keepAsksQuestion(_ copy: FirstRunCopy, count: Int) -> some View {
        GlassConfirmation(
            title: copy.uses.keepAsksTitle ?? copy.uses.sharing,
            message: copy.uses.keepAsksBody.map { FirstRunCopy.fill($0, ["count": String(count)]) } ?? "",
            actions: [
                .cancel(copy.passkey.cancel) { keepAsksCount = nil },
                GlassModalAction(copy.uses.keepAsksAll ?? "") { disclose(keepFolderAsks: false) },
                GlassModalAction(copy.uses.keepAsksKeep ?? "", isDefault: true) { disclose(keepFolderAsks: true) },
            ],
            onCancel: { keepAsksCount = nil })
    }

    private func choose(_ path: SharingPath?) {
        guard let path, path != SharingChoice.current(model.automaticGrant) else { return }
        refusal = nil
        switch path {
        case .askMe:
            working = true
            Task {
                if !(await model.setAskEachTime()) { refusal = Self.copy?.uses.sharingSaveFailed }
                working = false
            }
        case .automatic:
            switch SharingChoice.firstStep(asking: SharingChoice.askingFolders(model.projects)) {
            case .keepAsks(let count): keepAsksCount = count
            case .disclose(let keep): disclose(keepFolderAsks: keep)
            }
        }
    }

    private func disclose(keepFolderAsks keep: Bool) {
        keepAsksCount = nil
        keepFolderAsks = keep
        disclosure = SharingDisclosureFlow()
    }

    /// Both disclosures seen: the core decides whether the grant may be
    /// asked for and with which witness, as at setup's Start.
    private func finish(_ flow: SharingDisclosureFlow) {
        let request = SharingDisclosureFlow.grantRequest(
            flow.progress(connected: model.status.loggedIn, scopes: Set(model.status.consentScopes)))
        guard let request, request.ready else {
            refusal = Self.copy?.uses.sharingRefused
            return
        }
        working = true
        let keep = keepFolderAsks
        Task {
            if case .refused = await model.grantAutomatic(witness: request.witnessSigningAddress, keepFolderAsks: keep) {
                refusal = Self.copy?.uses.sharingRefused
            }
            working = false
        }
    }
}
