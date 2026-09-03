import Foundation

/// One place the redactor removed a value, as it appears in the preview
/// body.
public struct RedactionPlaceholder: Equatable {
    /// Where the token sits in the body it was scanned from.
    public let range: Range<String.Index>
    /// The raw label, as the redactor spelled it: `LOCAL_PATH`, `SECRET`.
    public let label: String
    /// Which distinct value of that label this is. The redactor mints one
    /// placeholder per value and reuses it, so the same ordinal appearing
    /// twice means the same original string appeared twice.
    public let ordinal: Int

    /// The label as a person reads it: "local path", "contextual entropy".
    public var display: String {
        label.lowercased().replacingOccurrences(of: "_", with: " ")
    }
}

/// Finding the redactor's placeholders in a preview body.
///
/// `DeterministicTraceRedactor` does not delete a matched value -- it
/// substitutes `<PRIVATE_<LABEL>_<n>>`, one token per distinct value, reused
/// wherever that value recurs. Those tokens have always been in the bytes
/// the ABI returns; the app just rendered them as ordinary text and the
/// contributor scrolled past them.
///
/// Marking them is the whole of "show me what got removed", and it is
/// better than a list because it also answers *where*. It needs no new
/// protocol field and no new content across any boundary: the token is
/// already what is on screen.
///
/// What it must not be allowed to imply: a region with no placeholder is
/// not a region with nothing sensitive in it. The detector scans every leaf
/// and the rewriter reaches only typed fields, so highlighting makes the app
/// look more thorough than it is. `ScrubbingCaveat`'s sentence is what says
/// so, and it belongs beside these marks rather than at the foot of the
/// screen.
public enum RedactionPlaceholders {
    /// Every placeholder in `body`, left to right.
    ///
    /// Deliberately strict about the shape: an uppercase label of letters,
    /// digits and underscores, then an underscore, then the ordinal, then
    /// `>`. A transcript can contain anything, including prose that looks
    /// approximately like a token, and marking a contributor's own sentence
    /// as a redaction would be a lie about what the scrubber did.
    public static func scan(_ body: String) -> [RedactionPlaceholder] {
        guard !body.isEmpty else { return [] }
        let pattern = /<PRIVATE_([A-Z0-9_]*[A-Z0-9])_([0-9]+)>/
        return body.matches(of: pattern).map { match in
            RedactionPlaceholder(
                range: match.range,
                label: String(match.output.1),
                ordinal: Int(match.output.2) ?? 0
            )
        }
    }
}
