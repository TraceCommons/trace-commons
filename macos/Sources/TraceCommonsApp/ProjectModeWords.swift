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

extension ProjectCopy {
    /// A project mode's name, for a picker option, a row's state or a
    /// summary: the core's, by wire mode. The accessor every macOS screen
    /// that names a folder mode uses, the glass Settings and onboarding
    /// included.
    static func modeChoiceLabel(_ mode: ProjectMode) -> String {
        ProjectModeWords.table?.label(for: mode) ?? ""
    }
}
