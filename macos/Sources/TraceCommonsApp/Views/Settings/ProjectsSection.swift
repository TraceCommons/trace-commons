import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The picker's options for one project row. Pure so the rule is testable:
/// the words are the core's contribution-mode table, by the daemon's own
/// mode string, and a mode the table does not name is not offered.
enum ProjectModeChoices {
    /// `dot` gives each mode a status dot, as the first run's Rules draws
    /// them (`RulesScreenLayout.dot`); Settings draws none.
    static func options(
        for modes: [ProjectMode], copy: ContributionModeCopy, dot: (ProjectMode) -> GlassStatus? = { _ in nil }
    ) -> [GlassPickerOption<ProjectMode>] {
        modes.compactMap { mode in
            copy.choice(for: mode.rawValue).map { GlassPickerOption($0.label, value: mode, dot: dot(mode)) }
        }
    }
}

/// Settings > Projects: setup's sharing choice, then a summary of how many
/// projects are in each mode and a way into the Manage sheet (owner design,
/// 2026-11-04). The list itself lives in the sheet, so this body stays a
/// few lines however many projects the daemon reports.
struct ProjectsSection: View {
    @EnvironmentObject private var model: AppModel
    @State private var managing = false

    static let modeCopy = ContributionModeCopy.decode(fromJSON: TCCoreCopy.contributionModeCopyJSON())
    static var words: ShellWordsCopy.ProjectsManager? { ShellWords.table?.projectsManager }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            // Setup's sharing choice, changeable here (#1753 A2), over the
            // folders it is the default for.
            SharingChoiceCard()
            GlassEyebrowCard(SettingsWords.projects, accessory: {
                if model.projectsRead == .answered, !model.projects.isEmpty {
                    Text(String(model.projects.count))
                        .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                        .foregroundStyle(GlassColor.textSecondary)
                        .monospacedDigit()
                }
            }) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    // The list defaults to empty, and a failed
                    // `list_projects` keeps it so; until that call answers,
                    // "none yet" would be a count nothing reported.
                    if model.projectsRead != .answered {
                        SettingsReadNotice(model.projectsRead, retry: model.refreshProjects)
                    } else if model.projects.isEmpty {
                        Text(SettingsLegacyWords.noProjectsYet)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                    } else if let words = Self.words, let copy = Self.modeCopy {
                        summary(words: words, copy: copy)
                    }
                    ProjectsErrorNotice()
                }
            }
        }
        .glassModal(isPresented: Binding(
            // Not presented without the words it draws.
            get: { managing && Self.words != nil && Self.modeCopy != nil },
            set: { if !$0 { managing = false } }
        )) {
            if let words = Self.words, let copy = Self.modeCopy {
                ProjectsManagerSheet(words: words, modeCopy: copy, onClose: { managing = false })
            }
        }
    }

    /// "Automatic 12  Ask 40  Never 176", then Manage.
    private func summary(words: ShellWordsCopy.ProjectsManager, copy: ContributionModeCopy) -> some View {
        let counts = ProjectsManager.counts(model.projects)
        let parts = ProjectsManagerSheet.filterModes.compactMap { mode in
            copy.choice(for: mode.rawValue).map {
                ShellWordsCopy.fill(words.modeCount, ["title": $0.label, "count": String(counts.count(for: mode))])
            }
        }
        return HStack(alignment: .center, spacing: GlassTokens.Space.s4) {
            HStack(spacing: GlassTokens.Space.s6) {
                ForEach(parts, id: \.self) { Text($0).monospacedDigit() }
            }
            .glassType(GlassTokens.TypeScale.caption)
            .foregroundStyle(GlassColor.textSecondary)
            .accessibilityElement(children: .combine)
            Spacer(minLength: 0)
            Button(words.manage) { managing = true }
                .buttonStyle(GlassButtonStyle(.secondary))
        }
    }

    /// What choosing a mode in Settings does, as the Traces tree decides
    /// it: arming always asks; Never asks when sessions are waiting, with
    /// their count, and is not made without the core's words for it; every
    /// other change is a direct call.
    enum Change: Equatable {
        case noop, apply, arm, ignore, unavailable
    }

    static func change(_ project: ProjectRow, to wanted: ProjectMode) -> Change {
        guard wanted != project.mode else { return .noop }
        if wanted == .autoUpload { return .arm }
        if wanted == .ignore, (project.pendingCount ?? 0) > 0 {
            return ignoreCopy(project) == nil ? .unavailable : .ignore
        }
        return .apply
    }

    /// The core's Ignore confirmation for this project and its waiting count.
    static func ignoreCopy(_ project: ProjectRow) -> ProjectIgnoreCopy? {
        ProjectIgnoreCopy.decode(fromJSON: TCCoreCopy.projectIgnoreCopyJSON(
            project: project.displayLabel, pending: project.pendingCount ?? 0))
    }
}

/// A refused mode change, unboxed, under the controls it is about (Ron,
/// 2026-10-09): on the summary card, and in the Manage sheet, where the
/// changes are made.
private struct ProjectsErrorNotice: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        if let error = model.lastActionError {
            HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s3) {
                GlassAlert(error)
                // An error is never undismissable, and is put away the way
                // every action message is: an x named by the banner's word,
                // never Traces' dismiss verb.
                Button { model.lastActionError = nil } label: {
                    Image(systemName: "xmark").imageScale(.small)
                }
                .buttonStyle(GlassButtonStyle(.link))
                .accessibilityLabel(ActionNoticeWords.dismissWord)
            }
            .padding(.top, GlassTokens.Space.s3)
        }
    }
}

/// Settings > Projects' Manage sheet (owner design, 2026-11-04): search,
/// a mode filter, projects sharing a name grouped under one header, and a
/// bulk change for the selected ones.
///
/// A glass modal rather than a window: it is the app's replacement for a
/// stock sheet, covers the Settings it was opened from, closes on Escape,
/// and stacks the arming and Never confirmations over itself.
///
/// Each row's control is the one the inline list had: `project.mode`, the
/// daemon's own answer, never local state; arming goes through the core's
/// confirmation and nothing else. A group's menu and the bulk menu offer
/// only Ask and Never (`ProjectsManager.bulkModes`): Automatic is a grant,
/// made one project at a time.
struct ProjectsManagerSheet: View {
    @EnvironmentObject private var model: AppModel
    let words: ShellWordsCopy.ProjectsManager
    let modeCopy: ContributionModeCopy
    let onClose: () -> Void

    @State private var query = ""
    /// Nil shows every mode.
    @State private var filter: ProjectMode?
    @State private var expanded: Set<String> = []
    @State private var selection: Set<String> = []
    @State private var armingCandidate: ProjectRow?
    /// A Never with sessions waiting, asked first in the core's words
    /// (#1146 `ProjectModeField`: "Ignore {project}?").
    @State private var ignoreCandidate: ProjectRow?
    /// A group or bulk Never with traces waiting, asked first in the same
    /// words, named by the group or by how many projects.
    @State private var bulkNever: BulkNever?

    struct BulkNever: Equatable {
        let ids: [String]
        let name: String
        let pending: Int
    }

    /// The filter's modes and the summary's, in the owner's order.
    static let filterModes: [ProjectMode] = [.autoUpload, .ask, .ignore]

    private var shown: [ProjectRow] {
        ProjectsManager.filter(model.projects, query: query, mode: filter)
    }

    private var searching: Bool { !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }

    var body: some View {
        let shown = shown
        let lines = ProjectsManager.lines(ProjectsManager.entries(shown), expanded: expanded, expandAll: searching)
        let selected = ProjectsManager.selected(selection, in: shown)
        GlassModal(title: SettingsWords.projects, onCancel: onClose) {
            VStack(alignment: .leading, spacing: 0) {
                HStack(alignment: .center, spacing: GlassTokens.Space.s4) {
                    GlassTextField(words.search, text: $query, prompt: words.search, showsLabel: false)
                    GlassSegmentedTabs(modeCopy.title, selection: $filter, segments: segments)
                        .fixedSize()
                }
                .padding(.horizontal, 18)
                .padding(.vertical, GlassTokens.Space.s4)
                ProjectsManagerList(lines: lines) { line in self.line(line) }
                    .frame(minHeight: 160, idealHeight: 420, maxHeight: 460)
                if lines.isEmpty {
                    Text(words.noMatches)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .padding(.horizontal, 18)
                        .padding(.bottom, GlassTokens.Space.s4)
                }
                if !selected.isEmpty {
                    bulkBar(selected)
                }
                ProjectsErrorNotice()
                    .padding(.horizontal, 18)
                    .padding(.bottom, GlassTokens.Space.s4)
            }
        }
        // Raised over the sheet: the layer the sheet sits in is a host.
        .glassModal(isPresented: Binding(
            // Not presented without the core's words: arming is never
            // confirmed against a sentence this shell wrote.
            get: { armingCandidate.flatMap(armingCopy) != nil },
            // Any dismissal clears the candidate.
            set: { if !$0 { armingCandidate = nil } }
        )) {
            if let project = armingCandidate, let copy = armingCopy(project) {
                // Not `.destructive`: arming destroys nothing and is
                // reversible from this same picker. Ron's #1146 heading,
                // line and buttons; the body is the arming disclosure.
                GlassConfirmation(
                    title: copy.settingsQuestion, subtitle: copy.settingsDescription.isEmpty ? nil : copy.settingsDescription, message: copy.body,
                    actions: [
                        .cancel(copy.settingsDecline) { armingCandidate = nil },
                        GlassModalAction(copy.settingsConfirm, isDefault: true) {
                            model.setProjectMode(project, mode: .autoUpload)
                            armingCandidate = nil
                        },
                    ],
                    onCancel: { armingCandidate = nil })
            }
        }
        .glassModal(isPresented: Binding(
            get: { ignoreCandidate.flatMap(ProjectsSection.ignoreCopy) != nil },
            set: { if !$0 { ignoreCandidate = nil } }
        )) {
            if let project = ignoreCandidate, let copy = ProjectsSection.ignoreCopy(project) {
                GlassConfirmation(
                    title: copy.title, message: copy.body,
                    actions: [
                        .cancel(copy.keep) { ignoreCandidate = nil },
                        .destructive(copy.button, help: copy.ignoreProjectTooltip) {
                            model.setProjectMode(project, mode: .ignore)
                            ignoreCandidate = nil
                        },
                    ],
                    onCancel: { ignoreCandidate = nil })
            }
        }
        .glassModal(isPresented: Binding(
            get: { bulkNever.flatMap(Self.bulkNeverCopy) != nil },
            set: { if !$0 { bulkNever = nil } }
        )) {
            if let request = bulkNever, let copy = Self.bulkNeverCopy(request) {
                GlassConfirmation(
                    title: copy.title, message: copy.body,
                    actions: [
                        .cancel(copy.keep) { bulkNever = nil },
                        .destructive(copy.button, help: copy.ignoreProjectTooltip) {
                            model.setProjectModes(request.ids, mode: .ignore)
                            selection.subtract(request.ids)
                            bulkNever = nil
                        },
                    ],
                    onCancel: { bulkNever = nil })
            }
        }
    }

    /// All, then each mode with how many projects are in it.
    private var segments: [GlassSegment<ProjectMode?>] {
        let counts = ProjectsManager.counts(model.projects)
        return [GlassSegment(words.all, value: nil)] + Self.filterModes.compactMap { mode in
            modeCopy.choice(for: mode.rawValue).map {
                GlassSegment($0.label, value: Optional(mode), badge: counts.count(for: mode))
            }
        }
    }

    /// The one Never confirmation's words (`ignore_project_copy`), named by
    /// `neverName`.
    static func bulkNeverCopy(_ request: BulkNever) -> ProjectIgnoreCopy? {
        ProjectIgnoreCopy.decode(fromJSON: TCCoreCopy.projectIgnoreCopyJSON(
            project: request.name, pending: request.pending))
    }

    /// What a group or bulk Never confirmation calls the projects it
    /// changes: the project's own name when it is one, else how many
    /// ("Set 3 projects to Never?"), for a group as for a selection (owner,
    /// 2026-11-04). Counted from the projects the change reaches, not those
    /// already Never.
    static func neverName(_ ids: [String], in rows: [ProjectRow], words: ShellWordsCopy.ProjectsManager) -> String {
        if ids.count == 1, let row = rows.first(where: { $0.id == ids[0] }) { return row.displayLabel }
        return ShellWordsCopy.fill(words.projectsCount, ["count": String(ids.count)])
    }

    /// Carry out a group or bulk change: Ask and Never only, a Never that
    /// clears waiting traces asks first, anything else is refused.
    private func run(_ rows: [ProjectRow], to mode: ProjectMode) {
        switch ProjectsManager.plan(rows, to: mode) {
        case .nothing, .refused: return
        case .apply(let ids):
            model.setProjectModes(ids, mode: mode)
            selection.subtract(ids)
        case .confirmNever(let ids, let pending):
            bulkNever = BulkNever(ids: ids, name: Self.neverName(ids, in: rows, words: words), pending: pending)
        }
    }

    /// The bulk menu's choices: Ask and Never, never Automatic.
    private var bulkOptions: [GlassPickerOption<ProjectMode>] {
        ProjectModeChoices.options(for: ProjectsManager.bulkModes, copy: modeCopy)
    }

    private func bulkBar(_ selected: [ProjectRow]) -> some View {
        HStack(alignment: .center, spacing: GlassTokens.Space.s4) {
            Text(ShellWordsCopy.fill(words.selected, ["count": String(selected.count)]))
                .glassType(GlassTokens.TypeScale.label.weight(.semibold))
            Spacer(minLength: 0)
            // An action menu: it shows its name, never a mode, and acts on
            // the choice.
            GlassPicker(
                words.setTo,
                selection: Binding<ProjectMode?>(
                    get: { nil },
                    set: { wanted in
                        guard let wanted else { return }
                        run(selected, to: wanted)
                    }),
                options: bulkOptions,
                placeholder: words.setTo)
        }
        .padding(.horizontal, 18)
        .padding(.vertical, GlassTokens.Space.s4)
        .overlay(alignment: .top) { GlassHairline(GlassColor.hairline) }
    }

    @ViewBuilder
    private func line(_ line: ProjectLine) -> some View {
        switch line {
        case .single(let project): row(project, member: false)
        case .member(let project, _): row(project, member: true)
        case .group(let group, let open): header(group, open: open)
        }
    }

    /// A group's header: a disclosure named by the group and its count, and
    /// a menu that sets every member to Ask or Never. A group whose members
    /// agree shows their mode; one whose members differ shows Mixed.
    private func header(_ group: ProjectGroup, open: Bool) -> some View {
        let name = ShellWordsCopy.fill(words.groupAccessibility, ["title": group.name, "count": String(group.members.count)])
        let shownMode = group.mode.flatMap { modeCopy.choice(for: $0.rawValue)?.label } ?? modeCopy.mixed
        return HStack(alignment: .center, spacing: GlassTokens.Space.s4) {
            Button {
                if expanded.contains(group.id) { expanded.remove(group.id) } else { expanded.insert(group.id) }
            } label: {
                HStack(spacing: GlassTokens.Space.s4) {
                    Image(systemName: "chevron.right")
                        .glassGlyph(10, weight: .semibold)
                        .foregroundStyle(GlassColor.textTertiary)
                        .rotationEffect(.degrees(open ? 90 : 0))
                        .frame(width: 12)
                    Text(group.name)
                        .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                    Text("(\(group.members.count))")
                        .foregroundStyle(GlassColor.textSecondary)
                        .monospacedDigit()
                    Spacer(minLength: 0)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(GlassPressStyle())
            // A native disclosure for assistive tech, so the system says
            // expanded or collapsed in the person's language.
            .accessibilityRepresentation {
                DisclosureGroup(isExpanded: Binding(
                    get: { open },
                    set: { if $0 { expanded.insert(group.id) } else { expanded.remove(group.id) } }
                )) { EmptyView() } label: { Text(name) }
            }
            GlassPicker(
                name,
                selection: Binding<ProjectMode?>(
                    // Only a mode this menu offers is shown as chosen; an
                    // all-Automatic group reads Automatic without offering it.
                    get: { group.mode.flatMap { ProjectsManager.bulkModes.contains($0) ? $0 : nil } },
                    set: { wanted in
                        guard let wanted else { return }
                        run(group.members, to: wanted)
                    }),
                options: bulkOptions,
                placeholder: shownMode)
        }
    }

    private func row(_ project: ProjectRow, member: Bool) -> some View {
        #if DEBUG
        OtherCollectionRowBodies.record("projects", project.id)
        #endif
        // `displayLabel`, not `projectLabel`: the bucket's own label is a
        // slug, and the row is recognised by the daemon's flag. Under a
        // group header only the suffix differs, so only it is shown.
        let label = member ? ProjectsManager.memberLabel(project) : project.displayLabel
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            HStack(alignment: .center, spacing: GlassTokens.Space.s4) {
                Toggle(isOn: Binding(
                    get: { selection.contains(project.id) },
                    set: { if $0 { selection.insert(project.id) } else { selection.remove(project.id) } }
                )) {
                    Text(label)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .toggleStyle(GlassCheckboxStyle())
                .accessibilityLabel(ShellWordsCopy.fill(words.selectAccessibility, ["title": project.displayLabel]))
                Spacer(minLength: 0)
                GlassPicker(
                    project.displayLabel,
                    selection: Binding<ProjectMode?>(
                        get: { project.mode },
                        set: { wanted in
                            guard let wanted else { return }
                            // Arming is a grant, so it is never silent;
                            // a Never that clears waiting sessions asks.
                            switch ProjectsSection.change(project, to: wanted) {
                            case .noop, .unavailable: return
                            case .arm: armingCandidate = project
                            case .ignore: ignoreCandidate = project
                            case .apply: model.setProjectMode(project, mode: wanted)
                            }
                        }),
                    options: ProjectModeChoices.options(for: project.offerableModes, copy: modeCopy),
                    placeholder: modeCopy.title)
            }
            if project.isUnresolvedBucket, let note = ProjectCopy.unresolvedBucketNote {
                Text(note)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.leading, member ? 22 : 0)
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

/// The Manage sheet's list: lazy, so a few hundred projects build only the
/// rows on screen.
struct ProjectsManagerList<Row: View>: View {
    let lines: [ProjectLine]
    @ViewBuilder let row: (ProjectLine) -> Row

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(Array(lines.enumerated()), id: \.element.id) { index, line in
                    GlassTableRow(first: index == 0) { row(line) }
                }
            }
            .padding(.horizontal, 18)
        }
    }
}
