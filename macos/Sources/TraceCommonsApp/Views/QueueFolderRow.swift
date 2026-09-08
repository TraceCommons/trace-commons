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
/// `Open` is the row's primary action and the only accented control on it.
/// `Submit all` is a peer, untinted, for the reason the card's `Submit` is:
/// the one-click design says Submit is not the primary action, and a filled
/// `Submit all` on the folder row was recommending exactly what the card
/// below it declines to recommend.
///
/// `Submit all` is shown at EVERY count, including one. The old rule hid it
/// at one because the row's own `Submit` was on the same screen and did the
/// same thing. Under drill-in it is a level down, so hiding it here would
/// mean opening a folder to do the thing the folder is offering. The rule
/// expired with the layout it was written for.
struct QueueFolderRow: View {
    let group: QueueGroup<QueueEntry>
    /// The eligibility branch tables, so this row's `Submit all` counts what
    /// it will actually send. See `submittableCount`.
    let eligibilityCalls: EligibilityCalls
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

    /// How many of this folder's sessions `Submit all` would send.
    ///
    /// **A GROUP-LEVEL SUBMIT MEANS "ALL ELIGIBLE", NEVER "ALL"** -- ruled
    /// 2026-09-08. The count on the button is the count that will leave, not
    /// `group.count`: a button promising more than it sends is the same
    /// press-then-discover shape the card's `Submit` was fixed for, and it
    /// is worse here because the contributor never saw the sessions.
    ///
    /// Equal to `group.count` for a folder with no eligibility question at
    /// all, so an invited contributor's row is unchanged.
    private var submittableCount: Int {
        EligibilitySurface.contributable(
            group.entries, eligibility: { $0.contributionEligibility }, calls: eligibilityCalls
        ).count
    }

    /// How many this button will leave behind, or zero.
    private var withheldCount: Int { group.count - submittableCount }

    /// What the button leaves out, or nothing.
    ///
    /// A COUNT AND NO REASON. Why a session cannot be sent is that row's own
    /// sentence one level in, drawn from the shared table; restating it here
    /// would be this shell authoring an eligibility claim about sessions it
    /// is not even showing.
    ///
    /// This sentence IS authored here, and it should not stay that way: the
    /// other two shells need the same line and a rename in the Rust will not
    /// reach this one. It belongs in the shared payload beside the four
    /// state sentences. `ShellWordingTests`' baseline for this file records
    /// the cost until it moves.
    private var withheldNote: String? {
        guard withheldCount > 0 else { return nil }
        return "^[\(withheldCount) session](inflect: true) here cannot be sent."
    }

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
                // Untinted, like the card's `Submit`: the one-click design
                // says Submit is availability, not a recommendation, and
                // the accent on this row belongs to opening the folder --
                // see `Open` at the trailing edge.
                //
                // The count is `submittableCount`, never `group.count`: this
                // button sends what it says and nothing more.
                Button("Submit all (\(submittableCount))", action: onSubmitAll)
                    .tint(.primary)
                    .disabled(submittableCount == 0)
                    .help("""
                    Submits every session in \(group.label) that can be sent. Each is \
                    scrubbed the same way a single Submit would be, and flagged \
                    sessions are included, not held back.
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
                // Disabled on the same condition as the plain button, and
                // it has to be said separately: this is a second route to
                // the same call, and a menu that stayed live beside a
                // disabled button would be the inert control #728 is
                // about -- pressing it would send nothing and say nothing.
                .disabled(submittableCount == 0)
                .help(VerdictCopy.submitAllAsTooltip)
                Spacer(minLength: TC.Space.m)
                // Never `.tcPrimaryAction()`: it sits beside a control that
                // uploads the very traces this removes, and two adjacent
                // actions that do opposite things must not look alike.
                Button(ProjectIgnoreCopy.buttonLabel) { confirmingIgnore = true }
                    .help(ProjectIgnoreCopy.tooltip)
                // The row's primary action, in the card's position (trailing,
                // default action last). It carries the accent that `Submit
                // all` used to: looking is what this product recommends,
                // and a folder row is offering a look at what is inside.
                Button("Open", action: onOpen)
                    .tcPrimaryAction()
                    .help("Opens \(group.label) to look at each session before deciding.")
            }

            // Drawn WHEREVER the button's count is short of the folder's,
            // and never separated from it: a button that says fewer
            // sessions than the row does, with nothing explaining the gap,
            // is its own small dishonesty.
            if let withheldNote {
                Text(withheldNote)
                    .font(TC.Font_.footnote)
                    .foregroundStyle(TC.inkSecondary)
                    .fixedSize(horizontal: false, vertical: true)
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
