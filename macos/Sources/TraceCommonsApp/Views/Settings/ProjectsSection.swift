import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The picker's options for one project row. Pure so the rule is testable:
/// the words are the core's contribution-mode table, by the daemon's own
/// mode string, and a mode the table does not name is not offered.
enum ProjectModeChoices {
    /// `dots` adds each mode's status dot, as the first run's Rules draws
    /// them (Ron's #1030 `RULE_DOTS`): Ask me ask, Automatic on, Never off.
    static func options(
        for modes: [ProjectMode], copy: ContributionModeCopy, dots: Bool = false
    ) -> [GlassPickerOption<ProjectMode>] {
        modes.compactMap { mode in
            copy.choice(for: mode.rawValue).map {
                GlassPickerOption($0.label, value: mode, dot: dots ? dot(for: mode) : nil)
            }
        }
    }

    static func dot(for mode: ProjectMode) -> GlassStatus {
        switch mode {
        case .ask: return .ask
        case .autoUpload: return .on
        case .ignore: return .off
        }
    }
}

/// One mode control per project, as `list_projects` reports them.
///
/// The list comes from `ProjectRow.offerableModes`, because the daemon
/// refuses `auto_upload` for the unresolvable bucket. The binding reads
/// `project.mode`, the daemon's own answer, never local state, so a mode the
/// daemon refuses leaves the picker showing what is in force. Arming goes
/// through the core's confirmation and nothing else. When the core's mode
/// table has not loaded no picker is drawn, so no mode is shown.
struct ProjectsSection: View {
    @EnvironmentObject private var model: AppModel
    @State private var armingCandidate: ProjectRow?

    private static let modeCopy = ContributionModeCopy.decode(fromJSON: TCCoreCopy.contributionModeCopyJSON())

    var body: some View {
        // The container is always present, so the dialog is attached
        // whether or not any project is drawn.
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            if let error = model.lastActionError {
                GlassNotice(tone: .outside) {
                    HStack(alignment: .top, spacing: GlassTokens.Space.s3) {
                        Text(error)
                            .fixedSize(horizontal: false, vertical: true)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        // An error is never undismissable, and is put away
                        // the way every action message is: an x named by
                        // the banner's word, never Traces' dismiss verb.
                        Button { model.lastActionError = nil } label: {
                            Image(systemName: "xmark").imageScale(.small)
                        }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .accessibilityLabel(ActionNoticeWords.dismissWord)
                    }
                }
            }
            GlassEyebrowCard(SettingsWords.projects) {
                VStack(alignment: .leading, spacing: 0) {
                    // The list defaults to empty, and a failed
                    // `list_projects` keeps it so; until that call answers,
                    // "none yet" would be a count nothing reported.
                    if model.projectsRead != .answered {
                        SettingsReadNotice(model.projectsRead, retry: model.refreshProjects)
                    } else if model.projects.isEmpty {
                        Text(SettingsLegacyWords.noProjectsYet)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                    }
                    ForEach(Array(model.projects.enumerated()), id: \.element.id) { index, project in
                        GlassTableRow(first: index == 0) { row(project) }
                    }
                }
            }
        }
        // Presented from the section: one dialog for the list, named by
        // whichever row is being armed.
        .glassModal(isPresented: Binding(
            // Not presented without the core's words: arming is never
            // confirmed against a sentence this shell wrote.
            get: { armingCandidate.flatMap(armingCopy) != nil },
            // Any dismissal clears the candidate.
            set: { if !$0 { armingCandidate = nil } }
        )) {
            if let project = armingCandidate, let copy = armingCopy(project) {
                // Not `.destructive`: arming destroys nothing and is
                // reversible from this same picker.
                GlassConfirmation(
                    title: copy.question, message: copy.body,
                    actions: [
                        .cancel(copy.decline) { armingCandidate = nil },
                        GlassModalAction(copy.confirm, isDefault: true) {
                            model.setProjectMode(project, mode: .autoUpload)
                            armingCandidate = nil
                        },
                    ],
                    onCancel: { armingCandidate = nil })
            }
        }
    }

    private func row(_ project: ProjectRow) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            HStack(alignment: .center, spacing: GlassTokens.Space.s4) {
                // `displayLabel`, not `projectLabel`: the bucket's own label
                // is a slug, and the row is recognised by the daemon's flag.
                Text(project.displayLabel)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 0)
                if let copy = Self.modeCopy {
                    GlassPicker(
                        project.displayLabel,
                        selection: Binding<ProjectMode?>(
                            get: { project.mode },
                            set: { wanted in
                                guard let wanted, wanted != project.mode else { return }
                                // Arming is a grant, so it is never silent;
                                // everything else is a direct call.
                                if wanted == .autoUpload {
                                    armingCandidate = project
                                } else {
                                    model.setProjectMode(project, mode: wanted)
                                }
                            }),
                        options: ProjectModeChoices.options(for: project.offerableModes, copy: copy),
                        placeholder: copy.title)
                }
            }
            if project.isUnresolvedBucket {
                Text(ProjectCopy.unresolvedBucketNote)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    /// The arming confirmation's words, from the core. No count is in hand
    /// here, so the offer's evidence line is not rendered.
    private func armingCopy(_ project: ProjectRow) -> ProjectArmingCopy? {
        ProjectArmingCopy.decode(fromJSON: TCCoreCopy.armingOfferCopyJSON(
            project: project.displayLabel,
            count: 0
        ))
    }
}
