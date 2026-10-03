import SwiftUI
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
                GlassEyebrowCard(model.routeDisclosureUnreadableCopy?.title ?? "") {
                    RouteDisclosureUnreadableGlassLine(
                        line: model.routeDisclosureUnreadableCopy?.panel,
                        fallback: model.routeDisclosureUnreadableCopy?.title)
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
                    GlassKeyValueList([
                        .init(witness.addressLabel, facts.url, mono: true),
                        .init(witness.signingLabel, facts.signingAddress, mono: true),
                        .init(witness.measurementsLabel, facts.pinnedMeasurements.joined(separator: "\n"), mono: true),
                    ])
                    sentence(witness.check)
                    if let classifier = witness.classifier { sentence(classifier) }
                    sentence(witness.origin)
                }
            }
            if let line = copy.attestedBodies { sentence(line) }
            if let line = copy.receipts { sentence(line) }
        }
        .accessibilityElement(children: .combine)
    }

    private func sentence(_ text: String) -> some View {
        Text(text)
            .glassType(GlassTokens.TypeScale.caption)
            .foregroundStyle(GlassColor.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
    }
}

/// The unreadable state: a glyph and words, so it survives greyscale, and
/// drawn even when the core's sentence for it could not be read, so the
/// panel is never simply empty.
struct RouteDisclosureUnreadableGlassLine: View {
    let line: String?
    var fallback: String?

    var body: some View {
        GlassStatusLabel(line ?? fallback ?? "", status: .ask)
            .accessibilityElement(children: .combine)
    }
}
