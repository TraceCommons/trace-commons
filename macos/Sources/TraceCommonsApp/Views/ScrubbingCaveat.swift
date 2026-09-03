import SwiftUI

/// How the limits of automatic scrubbing are told to a contributor.
///
/// ## The problem this type exists to solve
///
/// The sentence "Scrubbing is pattern-based. It misses things it hasn't seen
/// before." used to be printed, character for character, on every card in
/// the queue. It is the most important sentence in the product and repeating
/// it verbatim on every row is precisely how a person learns to stop reading
/// it: identical text in an identical position reads as furniture within
/// about two rows, and furniture is invisible.
///
/// Deleting it is not an option -- it is the sentence that makes every other
/// claim in this app credible. So it is not deleted, and it is not softened.
/// It is placed where it can still be heard, and the per-row slot it used to
/// occupy is given something that varies.
///
/// ## Where it lands now, in three places, each doing one job
///
/// 1. **On the card: a fact about THIS session, not a constant.**
///    `rowLine(redactionCount:)` says what scrubbing did here and what that
///    does not prove. A session where four things were removed and a session
///    where nothing matched now read differently, which is true, and which
///    means the line has to be read to be understood. The "nothing matched"
///    case is the one that most deserves a second look and it is the one the
///    old constant sentence flattened away entirely.
///
/// 2. **Under the list: the sentence itself, once per screen.**
///    `Self.canonical` verbatim, once, attached to the queue rather than to
///    any row -- it is a statement about the mechanism, and the mechanism is
///    a property of the list, not of a session.
///
/// 3. **At the moment of commitment: the sentence again, verbatim, directly
///    above Contribute.** This is the only irreversible click in the
///    product. A caveat that a person has scrolled past ten times is worth
///    less than a caveat sitting under their cursor at the instant it
///    matters, so it is repeated there deliberately, where repetition buys
///    something. Verbatim is the whole mechanism: a person who has read the
///    sentence under the list recognises it under the button, and a variant
///    wording there would read as a second, weaker message rather than as
///    the same promise being kept.
///
/// ## Copy provenance
///
/// `canonical` is the shared design spec's sentence, unchanged, and must
/// stay that way. The `rowLine` sentences are new: they were written for
/// this pass because the row needed something session-specific to say. They
/// concede exactly what the canonical sentence concedes -- pattern matching
/// only knows its patterns -- without claiming anything the daemon has not
/// actually reported.
///
/// `design-import/DESIGN-SPEC.md` §5.2 prints a different sentence in the
/// preview sheet's footer: "Scrubbing is pattern-based and may have missed
/// something. Look before you send." It concedes the same thing and adds an
/// instruction. It is deliberately NOT adopted, because point 3 above is an
/// argument about the sentence being identical in all three places, not
/// merely present in all three. Recorded here so the divergence is a
/// decision on the record rather than an oversight.
enum ScrubbingCaveat {
    /// The spec's sentence. Verbatim, load-bearing, not to be reworded.
    static let canonical =
        "Scrubbing is pattern-based. It misses things it hasn't seen before."

    /// What to say on one card, given what scrubbing actually did to it.
    static func rowLine(redactionCount: Int) -> String {
        redactionCount == 0
            ? "Nothing matched a pattern. That is not the same as nothing being there -- search it for anything you need to be sure isn't in it."
            : "Removed by pattern matching. Anything the patterns don't know is still in there."
    }

    /// A card where scrubbing found nothing is the one worth slowing down
    /// on, so it is the one case that gets a visible marker.
    static func tone(redactionCount: Int) -> TC.Tone {
        redactionCount == 0 ? .attention : .neutral
    }
}

/// The canonical sentence as a screen-level statement: one per list, tied to
/// the mechanism rather than to any single session.
struct ScrubbingCaveatNote: View {
    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: TC.Space.s) {
            Image(systemName: "info.circle")
                .imageScale(.small)
                .foregroundStyle(.tertiary)
                .accessibilityHidden(true)
            Text(ScrubbingCaveat.canonical)
                .font(TC.Font_.caption)
                .foregroundStyle(TC.inkSecondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// The canonical sentence at the point of no return, sitting against the
/// Contribute button rather than somewhere a person has already scrolled
/// past. Weighted to be read: a rule above it, and the amber this app
/// otherwise spends sparingly.
struct ScrubbingCaveatAtCommit: View {
    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: TC.Space.s) {
            Image(systemName: TC.Tone.attention.symbol)
                .imageScale(.small)
                .foregroundStyle(TC.Tone.attention.color)
                .accessibilityHidden(true)
            Text(ScrubbingCaveat.canonical)
                .font(TC.Font_.caption)
                .foregroundStyle(TC.inkSecondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Before you contribute. \(ScrubbingCaveat.canonical)")
    }
}
