import SwiftUI
import TCBridge
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
        VStack(alignment: .leading, spacing: TC.Space.l) {
            Text("What gets removed").font(TC.Font_.sectionTitle)

            if labels.isEmpty {
                // The honest fallback. The concession below still applies and
                // is arguably the more important half, so the sheet is not
                // empty even when the list cannot be produced.
                Text("The list of detectors could not be read from this build.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            } else {
                Text("Before a trace leaves this machine, these are found and replaced:")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)

                VStack(alignment: .leading, spacing: TC.Space.xs) {
                    ForEach(labels, id: \.self) { label in
                        HStack(alignment: .firstTextBaseline, spacing: TC.Space.xs) {
                            Image(systemName: "checkmark")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                            Text(label).font(TC.Font_.body)
                        }
                    }
                }
            }

            Text("Scrubbing is pattern-based. It misses things it hasn't seen before.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            HStack {
                Spacer()
                Button("Close") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(TC.Space.xxl)
        .frame(minWidth: 380, maxWidth: 460, alignment: .leading)
    }
}
