import SwiftUI
import TCShellCore

/// One folder in the queue's root list.
///
/// The folder name is this row's largest text. It used to be its smallest
/// -- `TC.Font_.meta` in `inkSecondary`, beside a primary-styled
/// `Submit all` -- so the line read as a button with a caption rather than
/// as a place with actions. At 149 waiting sessions that inversion is the
/// difference between a list you can scan and one you cannot.
///
/// `Submit all` is shown at EVERY count, including one. The old rule hid it
/// at one because the row's own `Submit` was on the same screen and did the
/// same thing. Under drill-in it is a level down, so hiding it here would
/// mean opening a folder to do the thing the folder is offering. The rule
/// expired with the layout it was written for.
struct QueueFolderRow: View {
    let group: QueueGroup<QueueEntry>
    let onOpen: () -> Void
    let onSubmitAll: () -> Void
    /// The opt-in bulk path: the same approval, carrying one verdict for
    /// every entry it covers. Separate from `onSubmitAll` so the plain
    /// one-click submit stays exactly one click and keeps sending no
    /// `outcome` at all.
    let onSubmitAllAs: (ContributorVerdict) -> Void
    let onIgnoreProject: () -> Void

    @State private var confirmingIgnore = false

    /// Display only, and empty against a daemon that predates the field --
    /// in which case the row shows its label alone rather than a blank line.
    private var path: String { group.entries.first?.projectPath ?? "" }

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.sm) {
            Button(action: onOpen) {
                HStack(alignment: .firstTextBaseline, spacing: TC.Space.s) {
                    VStack(alignment: .leading, spacing: TC.Space.xxs) {
                        Text(group.label)
                            .font(TC.Font_.cardTitle)
                            .foregroundStyle(TC.inkPrimary)
                        if !path.isEmpty {
                            Text(path)
                                .font(TC.Font_.meta)
                                .foregroundStyle(TC.inkSecondary)
                        }
                    }
                    Spacer(minLength: TC.Space.m)
                    Text("^[\(group.count) session](inflect: true)")
                        .font(TC.Font_.ledger)
                        .monospacedDigit()
                        .foregroundStyle(TC.inkSecondary)
                    Text(Format.bytes(group.bytes))
                        .font(TC.Font_.ledger)
                        .monospacedDigit()
                        .foregroundStyle(TC.inkTertiary)
                    QueueGlyph(glyph: .chevronRight, size: 11, color: TC.inkTertiary)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("\(group.label), \(group.count) waiting. Open.")

            HStack(spacing: TC.Space.s) {
                Button("Submit all (\(group.count))", action: onSubmitAll)
                    .tcPrimaryAction()
                    .help("""
                    Submits every session waiting in \(group.label). Each is scrubbed \
                    the same way a single Submit would be, and flagged sessions are \
                    included, not held back.
                    """)
                // Beside `Submit all`, never in front of it: answering the
                // outcome question for a whole folder is a choice a
                // contributor opts into, and the common path must not grow a
                // step because this exists. Never `.tcPrimaryAction()` --
                // one primary action per row, and it is the plain button.
                Menu(VerdictCopy.submitAllAs) {
                    ForEach(ContributorVerdict.allCases, id: \.rawValue) { option in
                        Button(option.label) { onSubmitAllAs(option) }
                    }
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
                .help(VerdictCopy.submitAllAsTooltip)
                Spacer(minLength: TC.Space.m)
                // Never `.tcPrimaryAction()`: it sits beside a control that
                // uploads the very traces this removes, and two adjacent
                // actions that do opposite things must not look alike.
                Button(ProjectIgnoreCopy.buttonLabel) { confirmingIgnore = true }
                    .help(ProjectIgnoreCopy.tooltip)
            }
        }
        .padding(TC.Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard()
        .confirmationDialog(
            ProjectIgnoreCopy.confirmationTitle(project: group.label),
            isPresented: $confirmingIgnore,
            titleVisibility: .visible
        ) {
            Button(ProjectIgnoreCopy.buttonLabel, role: .destructive, action: onIgnoreProject)
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(ProjectIgnoreCopy.confirmationBody(
                project: group.label,
                pendingCount: group.count
            ))
        }
    }
}
