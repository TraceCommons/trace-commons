import Foundation

/// The words for arming a project -- setting it to contribute without asking.
///
/// Arming is allowed from the app, but never silently. It is the strongest
/// thing this window can be set to do: after it, sessions from the project
/// are scrubbed and sent with nobody reading a preview first. That is a
/// different promise from ask-first, where a person is the last check, and
/// the confirmation has to say so rather than leaving it to be discovered
/// later from a history screen.
///
/// The wording is the Linux shell's, verbatim
/// (`crates/trace-commons-contributor-gtk/src/copy.rs`, the `Arming`
/// section), for the reason `ProjectCopy` gives about the bucket note: two
/// shells describing the same switch differently is worse than either
/// wording on its own, and a near-duplicate is how they start disagreeing.
/// Kept here rather than beside `SettingsView` so that neither surface
/// becomes the owner and the other the copy -- and so it is testable without
/// a display, which a SwiftUI view body is not.
public enum ProjectArmingCopy {
    /// Names the project, because a confirmation that says "this project"
    /// makes the reader look behind the sheet to check which one.
    public static func confirmationTitle(project: String) -> String {
        "Contribute from \(project) automatically?"
    }

    /// States the scrubbing, then what stops, then the way back, in that
    /// order. The scrubbing is stated first because it is the reassurance;
    /// the loss of review is stated second because it is the cost, and a
    /// confirmation that leads with reassurance and buries the cost is not
    /// asking a real question. The way back is last and unconditional --
    /// this is reversible, and a sheet that does not say so reads as a door
    /// that only opens one way.
    public static let confirmationBody = """
        Every future session in this project will be scrubbed and contributed \
        without asking you. You won't review them first.

        A session is sent a day after you last work on it, so there is time to \
        change your mind.

        You can turn this off at any time.
        """

    /// The confirm button carries the action rather than agreeing in the
    /// abstract: "OK" would make the reader reconstruct what they had just
    /// agreed to from the heading.
    public static let confirm = "Turn on automatic contributing"

    /// "Not now" rather than "Cancel". Declining here is a decision about
    /// this moment, not an error to back out of, and the row is unchanged
    /// either way.
    public static let cancel = "Not now"
}
