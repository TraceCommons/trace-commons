import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The answer to "what gets removed?", asked from the welcome screen.
///
/// A sheet rather than a seventh onboarding screen: this is reference
/// material read once, and the flow is six screens with one decision each --
/// a step that asks for no decision does not belong in it. An inline
/// disclosure was the other option and would push the promise and the
/// primary action down a page that does not scroll.
///
/// ## The list is generated
///
/// Every row comes from the scrubber's own detector table, by way of
/// `tc_scrub_detector_names`. Nothing here is transcribed. A hand-written
/// list of what is removed is a privacy claim that stops being true the day a
/// detector is added, and nothing in this app would fail when it did -- the
/// screen would simply keep describing an older build to someone deciding
/// whether to trust it.
///
/// Names only, never patterns: publishing the regexes would tell someone
/// trying to slip a secret past the scrubber exactly what to avoid.
///
/// The concession underneath is not decoration. A developer knows automatic
/// redaction is imperfect, and conceding it is what makes the list credible
/// rather than a promise the product cannot keep.
struct WhatGetsRemovedSheet: View {
    @Environment(\.dismiss) private var dismiss

    /// Injected so the sheet is renderable in a preview and a capture
    /// without the dylib answering; production passes nothing.
    var detectorNamesJSON: String? = TCScrubInfo.detectorNamesJSON()

    private var labels: [String] {
        guard let detectorNamesJSON else { return [] }
        return ScrubDetectors.labels(fromJSON: detectorNamesJSON)
    }

    var body: some View {
        GlassSheet(title: WhatGetsRemovedWords.title) {
            if labels.isEmpty {
                // The honest fallback. The concession below still applies and
                // is arguably the more important half, so the sheet is not
                // empty even when the list cannot be produced.
                Text(WhatGetsRemovedWords.couldNotRead)
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textSecondary)
            } else {
                Text(WhatGetsRemovedWords.foundAndReplaced)
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)

                VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                    ForEach(labels, id: \.self) { label in
                        HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s1) {
                            Image(systemName: "checkmark")
                                .glassType(GlassTokens.TypeScale.caption)
                                .foregroundStyle(GlassColor.textTertiary)
                                .accessibilityHidden(true)
                            Text(label)
                                .glassType(GlassTokens.TypeScale.body)
                                .foregroundStyle(GlassColor.textPrimary)
                        }
                        .accessibilityElement(children: .combine)
                    }
                }
            }

            Text(WhatGetsRemovedWords.patternBased)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textTertiary)
                .fixedSize(horizontal: false, vertical: true)

            HStack {
                Spacer()
                Button(WhatGetsRemovedWords.close) { dismiss() }
                    .buttonStyle(GlassButtonStyle(.glass))
                    .keyboardShortcut(.defaultAction)
            }
        }
        .frame(minWidth: 380, maxWidth: 460, alignment: .leading)
    }
}

/// This sheet's sentences, held verbatim from the legacy sheet.
enum WhatGetsRemovedWords {
    static let title = "What gets removed"
    static let couldNotRead = "The list of detectors could not be read from this build."
    static let foundAndReplaced = "Before a trace leaves this machine, these are found and replaced:"
    static let patternBased = "Scrubbing is pattern-based. It misses things it hasn't seen before."
    static let close = "Close"
}
