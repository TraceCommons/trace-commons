import Foundation

/// The Traces tab's resume notice (owner, 2026-11-04; the notice and its
/// words approved then): each time the tab is opened while the person has
/// paused uploads, Settings' paused sentence with Resume and Not now. Not
/// now hides it until the tab is next opened.
///
/// Never while uploads run, and never while they are held for another
/// reason -- the account needs attention, automatic contribution is held,
/// or this device is not enrolled -- where resuming would not send anything
/// and the notice would promise what it cannot do.
public enum PausedResumeNotice {
    /// The notice's id in the window's set of notices closed for now.
    public static let id = "paused-resume"

    /// Whether the notice shows.
    public static func shows(
        paused: Bool, loggedIn: Bool, needsAttention: Bool, contributionHeld: Bool,
        dismissedThisVisit: Bool
    ) -> Bool {
        paused && loggedIn && !needsAttention && !contributionHeld && !dismissedThisVisit
    }

    /// The window's closed notices after a tab is picked: opening the
    /// Traces tab is a new visit, so a Not now from the last one no longer
    /// hides it. Picking any other tab changes nothing.
    public static func closed(_ closed: Set<String>, afterOpeningTraces opened: Bool) -> Set<String> {
        opened ? closed.subtracting([id]) : closed
    }
}
