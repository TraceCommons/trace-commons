import Foundation

/// Every sentence this app says about claiming, editing and withdrawing a
/// public handle.
///
/// ## Where this copy comes from
///
/// `docs/superpowers/specs/2026-08-08-contributor-shell-shared-design.md`
/// specifies the consent-scope checkbox ("List my handle publicly as a
/// contributor") and nothing else about this surface: it draws no
/// handle-claiming screen and writes no sentence about what a claim did.
/// The Linux shell's `crates/trace-commons-contributor-gtk/src/copy.rs` is
/// therefore the source of truth here, and these constants mirror it word
/// for word. Two shells that word an outward-facing consent action
/// differently are two different promises about what becomes public, so
/// changes to either belong in both.
///
/// The one systematic difference is the dash: the GTK constants are written
/// with `--` because they are Rust source read in a terminal as often as on
/// a screen, and this app spells the same dash `—` as it does everywhere
/// else. No words differ.
enum PublicProfileCopy {
    // MARK: - The section (§5.6)

    static let heading = "Your public profile"
    static let listHandlePublicly = "List my handle publicly"
    static let footnote = """
    Attribution only — being listed grants no data use at all. Leaving the \
    roster removes you from future snapshots.
    """
    static let handleLabel = "Handle"
    static let bioLabel = "Bio — 280 bytes, plaintext, no HTML"
    static let saveProfile = "Save profile"
    static let leaveRoster = "Leave the roster"

    static func onRosterSince(_ date: String) -> String { "On the roster since \(date)" }

    // MARK: - The go-public dialog (§5.7)

    static let goPublicHeadline = "Put your handle on the public roster?"
    static let goPublicConfirm = "Go public"
    static let notNow = "Not now"
    static let publishedHeading = "What gets published"
    static let neverHeading = "What never does"
    static let goPublicAcknowledgement = """
    I understand my handle and aggregate counts become public. Leaving the \
    roster removes me from future snapshots.
    """
    static let goPublicFootnote = """
    Nothing is pre-checked, and Go public stays off until the acknowledgement \
    is on. This changes attribution only — it grants no data use.
    """

    /// The handle field inside the dialog. The panel's `handleLabel` names
    /// the same thing, but here the field is empty and has to say what to
    /// put in it, and "Handle" over an empty box does not.
    static let goPublicHandleLabel = "The handle to publish"
    /// The optional bio, said as optional: what `bioLabel` cannot carry is
    /// that leaving this empty is a complete answer rather than an
    /// unfinished form.
    static let goPublicBioLabel = "Bio, if you want one — 280 bytes, plaintext, no HTML"

    // MARK: - What a claim or a withdrawal actually did

    /// A claim the server accepted.
    static let published = "You're on the roster. Your handle and aggregate counts are public now."

    /// A claim the server accepted and this device then failed to write
    /// down.
    ///
    /// This is what `handle_persisted: false` means, and it is emphatically
    /// not a failed claim: the server has taken the handle, so the profile
    /// is public whatever happened on this machine afterwards. Telling a
    /// contributor their handle did not go up when it did is the one error
    /// this surface must never make — it is a false statement about a
    /// public, outward-facing act, and they would walk away believing they
    /// are unlisted. So this leads with the publication and describes the
    /// local loss for what it is.
    static let publishedNotCached = """
    You're on the roster — your handle and aggregate counts are public now. \
    This device couldn't keep its own copy of the profile, so this window \
    will show you as unlisted again until you save it once more. That doesn't \
    change anything about what is public.
    """

    /// A withdrawal the server accepted.
    static let leftRoster = """
    You've left the roster. Your handle isn't published any more, and future \
    snapshots won't include you.
    """

    /// The mirror of `publishedNotCached`: the row is gone from the server
    /// regardless, so the withdrawal is not in doubt — only what this
    /// window will show next.
    static let leftRosterNotCached = """
    You've left the roster — your handle isn't published any more, and future \
    snapshots won't include you. This device couldn't clear its own copy of \
    the profile, so this window may show the old handle again until it can.
    """

    /// A claim the daemon or the server refused, from the daemon's fixed
    /// label.
    ///
    /// Every branch says nothing was published, because in every one of them
    /// nothing was: the refusal happens before or instead of the `PUT`. The
    /// rules themselves are not re-implemented here — the daemon and the
    /// server share one copy of them — and the underlying error is never
    /// echoed, because the daemon deliberately does not forward it: it can
    /// carry a server response body or a URL.
    static func failureSentence(_ label: String) -> String {
        let reason: String
        switch label {
        case "handle-required":
            reason = "There's no handle in the box yet."
        case "handle-too-short":
            reason = "That handle is too short — it needs at least 3 characters."
        case "handle-too-long":
            reason = "That handle is too long — 32 characters at most."
        case "handle-invalid-character":
            reason = "A handle can only use letters, numbers, hyphens and underscores."
        case "handle-invalid-boundary":
            reason = "A handle has to start and end with a letter or a number."
        case "handle-consecutive-separators":
            reason = "A handle can't have two hyphens or underscores in a row."
        case "handle-reserved":
            reason = "That handle is reserved and can't be claimed."
        case "bio-too-long":
            reason = "That bio is over the 280-byte budget."
        case "bio-invalid-character":
            reason = "That bio has a character the roster doesn't take."
        case "bio-required-or-null", "bio-invalid":
            // Not reachable from this app — it always sends a bio key, null
            // or a string — and handled anyway, so a contract change
            // surfaces as a sentence rather than as the fallback.
            reason = "The bio wasn't sent in a form the roster takes."
        case "not-logged-in":
            reason = "This device isn't connected to Trace Commons."
        default:
            reason = "The request didn't go through."
        }
        return "\(reason) Nothing was published and nothing changed. You can try again."
    }

    /// The same, for a withdrawal: "nothing was published" is the wrong
    /// second clause when what failed was an attempt to *un*-publish, and a
    /// contributor who read it could conclude they had been taken off the
    /// roster when they are still on it.
    static func leaveFailureSentence(_ label: String) -> String {
        let reason = label == "not-logged-in"
            ? "This device isn't connected to Trace Commons."
            : "The request didn't go through."
        return """
        \(reason) You're still on the roster and your handle is still \
        published. You can try again.
        """
    }
}

/// The assertions this copy has to keep passing, checked at render time.
///
/// There is no Swift test target in this package, so the alternative was
/// assertions nobody runs -- the same reason `WithdrawalCopyCheck` is
/// rendered rather than tested. The Linux shell asserts the same properties
/// in `copy.rs`'s unit tests; this is that suite, in the place it can
/// actually run. Empty in every healthy build.
enum PublicProfileCopyCheck {
    static func failures() -> [String] {
        var problems: [String] = []

        // `handle_persisted: false` is a failed local cache write, not a
        // failed claim: the server has already taken the handle. Both
        // sentences must therefore open by saying the contributor is on the
        // roster, and neither may read as a refusal.
        for sentence in [PublicProfileCopy.published, PublicProfileCopy.publishedNotCached] {
            if !sentence.hasPrefix("You're on the roster") {
                problems.append("a published profile is not reported as published")
            }
            let lower = sentence.lowercased()
            for forbidden in ["couldn't publish", "failed", "wasn't published", "nothing changed"]
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
        // these cases the handle did not go up -- and that has to be
        // distinguishable from the published-but-uncached case above.
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
        ] where !PublicProfileCopy.failureSentence(label).contains("Nothing was published") {
            problems.append("\(label) does not say the handle stayed private")
        }

        // "Nothing was published" is false comfort after a failed
        // withdrawal: the handle is published, which is the problem.
        for label in ["not-logged-in", "profile-withdraw-failed"] {
            let sentence = PublicProfileCopy.leaveFailureSentence(label)
            if sentence.contains("Nothing was published")
                || !sentence.contains("still on the roster")
            {
                problems.append("a failed withdrawal does not say the listing survived")
            }
        }

        // The daemon never forwards the underlying error -- it can carry a
        // response body or a URL -- and this mapping must not invent a place
        // to put one either.
        let unknown = PublicProfileCopy.failureSentence("https://ingest.example/v1/community/profile")
        if unknown.contains("https://") || unknown != PublicProfileCopy.failureSentence("other") {
            problems.append("an unknown label is echoed rather than mapped")
        }

        return problems
    }
}
