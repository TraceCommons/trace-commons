import Foundation
import TCShellCore

/// Every sentence this app says about claiming, editing and withdrawing a
/// public handle, read from the core's table
/// (`shell_words_copy::public_profile_words`, through `ShellWords`).
///
/// The wording follows Ron's #1146 profile screens (owner ruling,
/// 2026-10-06) where #1146 has a counterpart; the consent and outcome
/// sentences it changed are marked DRAFT in the core until approved. With
/// no table every word is empty, and `PublicProfileCopyCheck` draws the
/// defect notice rather than a silent screen.
enum PublicProfileCopy {
    private static var words: ShellWordsCopy.PublicProfile? { ShellWords.table?.publicProfile }

    // MARK: - The section (§5.6)

    static var heading: String { words?.heading ?? "" }
    static var listHandlePublicly: String { words?.listHandlePublicly ?? "" }
    static var footnote: String { words?.footnote ?? "" }
    static var handleLabel: String { words?.handle ?? "" }
    static var bioLabel: String { words?.bio ?? "" }
    /// Re-publishes the profile as it stands (#1146's "Update profile").
    static var saveProfile: String { words?.updateProfile ?? "" }
    /// Takes the handle off the roster (#1146's "Withdraw").
    static var leaveRoster: String { words?.withdraw ?? "" }

    static func onRosterSince(_ date: String) -> String {
        ShellWords.fill(words?.onRosterSince ?? "", ["date": date])
    }

    // MARK: - The go-public dialog (§5.7)

    static var goPublicHeadline: String { words?.goPublicHeadline ?? "" }
    static var goPublicDescription: String { words?.goPublicDescription ?? "" }
    static var goPublicConfirm: String { words?.goPublic ?? "" }
    static var goingPublic: String { words?.goingPublic ?? "" }
    static var notNow: String { words?.notNow ?? "" }
    static var publishedHeading: String { words?.publishedHeading ?? "" }
    static var publishedLines: [String] { words?.publishedLines ?? [] }
    static var neverHeading: String { words?.neverHeading ?? "" }
    static var neverLines: [String] { words?.neverLines ?? [] }
    static var goPublicAcknowledgement: String { words?.acknowledgement ?? "" }
    static var goPublicFootnote: String { words?.goPublicFootnote ?? "" }

    /// The handle field inside the dialog: here the field is empty and has
    /// to say what to put in it.
    static var goPublicHandleLabel: String { words?.goPublicHandle ?? "" }
    /// The optional bio, said as optional.
    static var goPublicBioLabel: String { words?.goPublicBio ?? "" }

    // MARK: - What a claim or a withdrawal actually did

    /// A claim the server accepted.
    static var published: String { words?.published ?? "" }

    /// A claim the server accepted and this device then failed to write
    /// down. Emphatically not a failed claim: the server has taken the
    /// handle, so the profile is public whatever happened here afterwards,
    /// and the sentence leads with that.
    static var publishedNotCached: String { words?.publishedNotCached ?? "" }

    /// A withdrawal the server accepted.
    static var leftRoster: String { words?.leftRoster ?? "" }

    /// The mirror of `publishedNotCached`: the row is gone from the server
    /// regardless; only what this window shows next is in doubt.
    static var leftRosterNotCached: String { words?.leftRosterNotCached ?? "" }

    /// A claim the daemon or the server refused, from the daemon's fixed
    /// label. Every branch says nothing was published, because in every one
    /// of them nothing was. The label itself is never echoed: it can carry
    /// a server response body or a URL.
    static func failureSentence(_ label: String) -> String {
        guard let words else { return "" }
        guard let reason = words.failureReasons[label] else { return words.failureDefault }
        return ShellWords.fill(words.failure, ["reason": reason])
    }

    /// The same, for a withdrawal: the handle is still published, which is
    /// the problem, so it never says "not published".
    static func leaveFailureSentence(_ label: String) -> String {
        guard let words else { return "" }
        let reason = label == "not-logged-in" ? words.leaveNotConnected + " " : ""
        return ShellWords.fill(words.leaveFailure, ["reason": reason])
    }

    /// The defect notice's title when `PublicProfileCopyCheck` fails; the
    /// core's unavailable word when the table did not decode (the likeliest
    /// defect), never "".
    static var wordingDefect: String? { ShellWords.defectTitle(words?.wordingDefect) }
}

/// The assertions this copy has to keep passing, checked at render time on
/// what this build decoded. The core asserts the same properties of its
/// table (`shell_words_copy`'s tests). Empty in every healthy build.
enum PublicProfileCopyCheck {
    static func failures() -> [String] {
        var problems: [String] = []
        guard ShellWords.table != nil else {
            return ["the public-profile wording did not arrive from the core"]
        }

        // `handle_persisted: false` is a failed local cache write, not a
        // failed claim: the server has already taken the handle. Both
        // sentences must say the profile is public, and neither may read as
        // a refusal.
        for sentence in [PublicProfileCopy.published, PublicProfileCopy.publishedNotCached] {
            let lower = sentence.lowercased()
            if !lower.contains("public") {
                problems.append("a published profile is not reported as published")
            }
            for forbidden in ["couldn't publish", "failed", "wasn't published", "not published", "nothing changed"]
            where lower.contains(forbidden) {
                problems.append("a published profile reads as a failure (\(forbidden))")
            }
        }
        if PublicProfileCopy.published == PublicProfileCopy.publishedNotCached {
            problems.append("an uncached claim says nothing about the local copy")
        }

        // The mirror: the row is gone from the server whether or not the
        // local clear stuck.
        for sentence in [PublicProfileCopy.leftRoster, PublicProfileCopy.leftRosterNotCached]
        where !sentence.hasPrefix("You've left the roster") {
            problems.append("a completed withdrawal is not reported as completed")
        }

        // A refusal happens before or instead of the PUT, so in every one of
        // these cases the handle did not go up.
        for label in [
            "handle-required",
            "handle-too-short",
            "handle-too-long",
            "handle-invalid-character",
            "handle-invalid-boundary",
            "handle-consecutive-separators",
            "handle-reserved",
            "bio-too-long",
            "bio-invalid-character",
            "not-logged-in",
            "profile-update-failed",
            "a-label-nobody-has-written-yet"
        ] where !PublicProfileCopy.failureSentence(label).contains("not published") {
            problems.append("\(label) does not say the handle stayed private")
        }

        // "Not published" is false comfort after a failed withdrawal: the
        // handle is published, which is the problem.
        for label in ["not-logged-in", "profile-withdraw-failed"] {
            let sentence = PublicProfileCopy.leaveFailureSentence(label)
            if sentence.contains("not published") || !sentence.contains("still on the roster") {
                problems.append("a failed withdrawal does not say the listing survived")
            }
        }

        // The daemon never forwards the underlying error, and this mapping
        // must not invent a place to put one either.
        let unknown = PublicProfileCopy.failureSentence("https://ingest.example/v1/community/profile")
        if unknown.contains("https://") || unknown != PublicProfileCopy.failureSentence("other") {
            problems.append("an unknown label is echoed rather than mapped")
        }

        return problems
    }
}
