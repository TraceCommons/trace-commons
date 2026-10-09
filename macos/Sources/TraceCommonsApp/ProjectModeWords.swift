import TCBridge
import TCShellCore

/// The core's names for the folder modes, decoded once from the pill's table
/// (`tc_contribution_mode_copy_json`, `project_copy::FOLDER_MODE_LABELS`).
///
/// Owner decision, 2026-10-02: a folder mode has one name on every surface --
/// the menu-bar pill, Settings, onboarding, the arming and override words --
/// in every shell. This shell spells none of them. With no table a name is
/// empty, never a Swift fallback.
enum ProjectModeWords {
    static let table: ContributionModeCopy? = ContributionModeCopy.decode(fromJSON: TCCoreCopy.contributionModeCopyJSON())
}

/// The Traces tree's words (`MonitorTreeCopy`), decoded once, for the
/// surfaces that draw them without a `TracesStore` at hand: Settings, first
/// run and the window roots' folder mark.
enum TracesTreeWords {
    static let table: MonitorTreeCopy? = MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON())?.tree
}

extension ProjectCopy {
    /// A project mode's name, for a picker option, a row's state or a
    /// summary: the core's, by wire mode. The accessor every macOS screen
    /// that names a folder mode uses, the glass Settings and onboarding
    /// included.
    static func modeChoiceLabel(_ mode: ProjectMode) -> String {
        ProjectModeWords.table?.label(for: mode) ?? ""
    }

    /// The line under the bucket of sessions whose folder could not be
    /// resolved, in the core's words: a statement of what the daemon does,
    /// not an apology. Nil with no table, and then nothing is drawn.
    static var unresolvedBucketNote: String? { TracesTreeWords.table?.unresolvedBucketNote }
}
