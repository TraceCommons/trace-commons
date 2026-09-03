import Foundation

/// One marker in a transcript, with whatever can honestly be said about it.
public struct RedactionMark: Equatable {
    /// Where the token sits in the text it was scanned from. Always one of
    /// `TranscriptMarkerScan`'s spans -- the chipper's own -- so a name can
    /// never land somewhere no chip was drawn.
    public let range: Range<String.Index>
    /// What was removed, where the token says so. Nil for the bare
    /// `[REDACTED]`, which records that something left and not what. Never
    /// invented.
    public let category: String?
    /// Which distinct value this is, where the token is numbered. Nil for
    /// every fixed token, and never filled in with a placeholder number:
    /// the fixed forms carry no index because they do not track values.
    public let ordinal: Int?
    /// Whether the identical numbered token appeared earlier in this text,
    /// which means the identical original value did.
    ///
    /// False for every fixed token, always. Two `[REDACTED]`s say nothing
    /// about each other, and calling the second one a repeat would claim a
    /// fact nobody has.
    public let isRepeat: Bool
    /// What this mark is called, for a reader who cannot see it or cannot
    /// read the token.
    public let name: String
}

/// Naming the redaction markers the transcript already draws.
///
/// The marking is not new. `TranscriptMarkerScan` finds these spans and
/// `TranscriptMarkers` chips them, and has for some time -- the GTK and
/// Windows shells carry the identical pattern and do the same. What was
/// missing is that every chip drew as the same anonymous block, while the
/// tokens themselves carry a label, an ordinal, or a category.
///
/// So this adds naming and nothing else. It runs over
/// `TranscriptMarkerScan`'s spans rather than a pattern of its own, which is
/// the point: a second scan could disagree with the one the chips and the
/// chunker share, and the chunker depends on that scan to avoid cutting a
/// marker in half.
///
/// **What it must not be allowed to imply.** Markers appear where redaction
/// REWROTE a typed field. The detector scans every leaf; the rewriter does
/// not reach all of them. A region with no marker is not a region with
/// nothing sensitive in it. Naming the marks makes the app look more
/// thorough than it is, which is exactly when `ScrubbingCaveat`'s sentence
/// earns its place beside them.
public enum RedactionMarks {
    /// What the bare `[REDACTED]` can be called. It records that something
    /// left, and there is nothing else in the token to say.
    public static let unnamed = "something removed"

    /// The suffix on a numbered token whose value was already marked
    /// earlier. A reader cannot see that without scrolling back, which is
    /// the whole reason it is worth saying.
    static let repeatSuffix = ", the same value as an earlier mark"

    /// Every marker in `text`, left to right, named.
    public static func scan(_ text: String) -> [RedactionMark] {
        var seen: Set<String> = []
        return TranscriptMarkerScan.spans(in: text).map { range in
            let token = String(text[range])
            let (category, ordinal) = classify(token)
            // Only a numbered token identifies a value, so only a numbered
            // token can repeat one.
            let isRepeat = ordinal != nil && !seen.insert(token).inserted
            return RedactionMark(
                range: range,
                category: category,
                ordinal: ordinal,
                isRepeat: isRepeat,
                name: name(category: category, isRepeat: isRepeat)
            )
        }
    }

    /// `text` with every marker replaced by its name.
    ///
    /// This is what a chunk reads as aloud. Left as it is, a screen reader
    /// spells `<PRIVATE_LOCAL_PATH_1>` out as punctuation and capitals in
    /// the middle of a sentence; named, it says what happened there.
    public static func spoken(_ text: String) -> String {
        var out = ""
        var cursor = text.startIndex
        for mark in scan(text) {
            out += text[cursor..<mark.range.lowerBound]
            out += mark.name
            cursor = mark.range.upperBound
        }
        out += text[cursor...]
        return out
    }

    /// What one token says about itself.
    ///
    /// Four forms, and only the first two name anything:
    ///
    /// * `<PRIVATE_{LABEL}_{n}>` -- minted by `apply_placeholder_regex` for
    ///   exactly two labels, `local_path` and `private_email`. The only form
    ///   with an ordinal.
    /// * `[REDACTED:{label}]` -- `tool_sensitive_field{:action}` and every
    ///   `privacy_filter:{label}`. Names its category, carries no index.
    /// * `<REDACTED_PRIVATE_KEY>` -- PEM keys. Named here for completeness;
    ///   `TranscriptMarkerScan` does not match it, so no shell chips it.
    /// * `[REDACTED]` -- plain secrets and `sensitive_field`. Says that
    ///   something left and not what, which is the limit on how well this
    ///   one can ever be labelled.
    public static func classify(_ token: String) -> (category: String?, ordinal: Int?) {
        if let placeholder = RedactionPlaceholders.scan(token).first,
           String(token[placeholder.range]) == token
        {
            return (placeholder.display, placeholder.ordinal)
        }
        if token == "<REDACTED_PRIVATE_KEY>" {
            return ("private key", nil)
        }
        let labelled = "[REDACTED:"
        if token.hasPrefix(labelled), token.hasSuffix("]") {
            let label = token.dropFirst(labelled.count).dropLast()
            guard !label.isEmpty else { return (nil, nil) }
            return (label.replacingOccurrences(of: "_", with: " "), nil)
        }
        return (nil, nil)
    }

    static func name(category: String?, isRepeat: Bool) -> String {
        let what = category.map { "\($0) removed" } ?? unnamed
        return isRepeat ? what + repeatSuffix : what
    }
}
