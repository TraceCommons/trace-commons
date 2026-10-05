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
    var navigation: MainWindowNavigation?

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            if let copy = model.privateInferenceCopy {
                GlassEyebrowCard(copy.settingsTitle) {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                        Text(copy.settingsMoved)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                        // The label is the destination's own, so the sidebar
                        // and this pointer can never name it differently.
                        Button(copy.destination) {
                            navigation?.section = .privateInference
                        }
                        .buttonStyle(GlassButtonStyle(.link))
                    }
                }
            }
            switch model.routeDisclosureState {
            case .shown(let disclosure):
                GlassEyebrowCard(disclosure.copy.title) {
                    RouteDisclosureGlassBody(disclosure: disclosure)
                }
            case .loading:
                ProgressView().controlSize(.small)
            case .unreadable:
                let parts = RouteDisclosureUnreadableGlassLine.parts(
                    panel: model.routeDisclosureUnreadableCopy?.panel,
                    title: model.routeDisclosureUnreadableCopy?.title,
                    unknown: RouteDisclosureUnreadableGlassLine.unknown)
                GlassEyebrowCard(parts.title) {
                    RouteDisclosureUnreadableGlassLine(line: parts.line)
                }
            }
        }
        .onAppear { model.refreshRouteDisclosure() }
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
/// card's title carries the words when the panel sentence is missing, down to
/// the core's own "unknown" word, so the panel is never a bare dot; the line
/// then draws the glyph alone rather than repeat the title.
struct RouteDisclosureUnreadableGlassLine: View {
    let line: String?

    /// The core's word for an answer it does not have.
    static let unknown: String? = MonitorScreensCopy.decode(fromJSON: TCCoreCopy.monitorScreensCopyJSON())?.unknown

    /// The card's title and the line inside it. The title is the core's
    /// unreadable title, then its "unknown" word, then a dash (not a
    /// sentence); the line is the panel sentence, and nil when that is
    /// missing or would repeat the title, so the same words are never drawn
    /// twice.
    static func parts(panel: String?, title: String?, unknown: String?) -> (title: String, line: String?) {
        let heading = title ?? unknown ?? "\u{2014}"
        return (heading, panel == heading ? nil : panel)
    }

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s3) {
            Image(systemName: "exclamationmark.triangle.fill")
                .imageScale(.small)
                .foregroundStyle(GlassColor.textSecondary)
                .accessibilityHidden(true)
            if let words = line {
                GlassStatusLabel(words, status: .ask)
                    .accessibilityLabel(words)
            }
        }
        .accessibilityElement(children: .combine)
        // Without a line the card's title has already said it.
        .accessibilityHidden(line == nil)
    }
}
