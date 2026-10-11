import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// Where the Private AI switch moved to, and the route disclosure: the raw
/// send, both enclaves, and where the witness came from. The pointer draws
/// nothing if the core's words did not arrive; the disclosure always draws
/// something, so the outer container is always present and carries the
/// refresh. Unreadable is said as such and never drawn as some other route.
struct PrivateAISection: View {
    @EnvironmentObject private var model: AppModel
    /// Where the pointer goes when Settings is the Monitor's modal: it closes
    /// the modal and opens the Inference tab (Ron's #1146). With none, the
    /// pointer opens the Monitor at Inference (`OpenMonitor`).
    var onPointer: (() -> Void)? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            if let copy = model.privateInferenceCopy {
                // The whole card is the pointer, a chevron at its end, as
                // Home's cards are (owner, 2026-10-10: no Open button). It
                // lands on the Inference inspector, where the switch is; the
                // hint is the destination's own name, so the tab and this
                // pointer can never name it differently.
                GlassEyebrowCard(copy.settingsTitle, action: openDestination, chevron: true) {
                    Text(copy.settingsMoved)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .accessibilityHint(copy.destination)
            }
            switch model.routeDisclosureState {
            case .shown(let disclosure):
                GlassEyebrowCard(disclosure.copy.title) {
                    RouteDisclosureGlassBody(disclosure: disclosure)
                }
            case .loading:
                GlassSpinner(standalone: true)
            case .unreadable:
                GlassEyebrowCard(RouteDisclosureUnreadableGlassLine.text(
                    line: nil, fallback: model.routeDisclosureUnreadableCopy?.title,
                    unknown: RouteDisclosureUnreadableGlassLine.unknown)) {
                    RouteDisclosureUnreadableGlassLine(
                        line: model.routeDisclosureUnreadableCopy?.panel,
                        fallback: model.routeDisclosureUnreadableCopy?.title)
                }
            }
        }
        .onAppear { model.refreshRouteDisclosure() }
    }

    private func openDestination() {
        if let onPointer {
            onPointer()
        } else {
            OpenMonitor.request(.inference)
        }
    }
}

/// Every fact is the daemon's and every sentence the shared crate's; this
/// only lays them out. A block is drawn only when the core sent words for
/// it. Standalone so the Traces preview can reuse it.
struct RouteDisclosureGlassBody: View {
    let disclosure: RouteDisclosure

    var body: some View {
        let copy = disclosure.copy
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            sentence(copy.route)
            if let line = copy.localFilter { sentence(line) }
            if let facts = disclosure.facts.witness, let witness = copy.witness {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    Text(witness.heading)
                        .glassType(GlassTokens.TypeScale.eyebrow)
                        .foregroundStyle(GlassColor.textTertiary)
                    GlassKeyValueList(Self.witnessItems(witness: witness, facts: facts))
                    sentence(witness.check)
                    if let classifier = witness.classifier { sentence(classifier) }
                    sentence(witness.origin)
                }
            }
            if let line = copy.attestedBodies { sentence(line) }
            if let line = copy.receipts { sentence(line) }
        }
        // Each paragraph and each witness row is its own VoiceOver stop.
        .accessibilityElement(children: .contain)
    }

    /// The measurements row is left out when nothing is pinned, never drawn
    /// as a label over a blank value.
    static func witnessItems(
        witness: RouteDisclosure.WitnessCopy, facts: RouteDisclosure.WitnessFacts
    ) -> [GlassKeyValueList.Item] {
        var items: [GlassKeyValueList.Item] = [
            .init(witness.addressLabel, facts.url, mono: true),
            .init(witness.signingLabel, facts.signingAddress, mono: true),
        ]
        if !facts.pinnedMeasurements.isEmpty {
            items.append(.init(witness.measurementsLabel, facts.pinnedMeasurements.joined(separator: "\n"), mono: true))
        }
        return items
    }

    private func sentence(_ text: String) -> some View {
        Text(text)
            .glassType(GlassTokens.TypeScale.caption)
            .foregroundStyle(GlassColor.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
    }
}

/// The unreadable state: a shape glyph and words, so it survives greyscale,
/// and drawn even when the core's sentences for it could not be read. The
/// last resort is the core's own "unknown" word, so the panel is never a
/// bare dot.
struct RouteDisclosureUnreadableGlassLine: View {
    let line: String?
    var fallback: String?

    /// The core's word for an answer it does not have.
    static let unknown: String? = MonitorScreensCopy.decode(fromJSON: TCCoreCopy.monitorScreensCopyJSON())?.unknown

    /// A dash, not a sentence, only if the core's word could not be read.
    static func text(line: String?, fallback: String?, unknown: String?) -> String {
        line ?? fallback ?? unknown ?? "\u{2014}"
    }

    var body: some View {
        let words = Self.text(line: line, fallback: fallback, unknown: Self.unknown)
        HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s3) {
            GlassWarningGlyph()
                .foregroundStyle(GlassColor.textSecondary)
            GlassStatusLabel(words, status: .ask)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(words)
    }
}
