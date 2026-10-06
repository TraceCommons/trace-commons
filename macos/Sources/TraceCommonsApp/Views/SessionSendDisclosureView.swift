import SwiftUI
import TCDesign
import TCShellCore

// The review sheet's per-session send disclosure, moved from the retired
// `RouteDisclosureView.swift` when Settings' route-disclosure panel moved to
// glass (`Views/Settings/PrivateAISection.swift`). Its unreadable state is
// that panel's `RouteDisclosureUnreadableGlassLine`.

/// For one session in the review sheet: the size before redaction and after,
/// where each goes, and -- where the witness reviewed it -- what it was
/// checked against. The sizes are the preview's own; raw text is never shown,
/// because it does not cross the preview boundary.
struct SessionSendDisclosureView: View {
    @EnvironmentObject private var model: AppModel
    let entry: QueueEntry
    let rawSessionBytes: Int
    let wouldSendBytes: Int

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let disclosure = model.routeDisclosure {
                let session = disclosure.copy.session
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    eyebrow(session.heading)
                    Text("\(session.beforeLabel) · \(Format.bytes(rawSessionBytes))")
                        .glassType(GlassTokens.TypeScale.bodyStrong)
                        .foregroundStyle(GlassColor.textPrimary)
                    caption(session.beforeLine)
                    if let line = disclosure.copy.localFilter { caption(line) }
                    Text("\(session.afterLabel) · \(Format.bytes(wouldSendBytes))")
                        .glassType(GlassTokens.TypeScale.bodyStrong)
                        .foregroundStyle(GlassColor.textPrimary)
                    caption(session.afterLine)
                    if disclosure.sendsToWitness { caption(disclosure.copy.route) }
                    if let detail = model.certificateDetails[entry.entryID] {
                        VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                            Text(detail.heading)
                                .glassType(GlassTokens.TypeScale.bodyStrong)
                                .foregroundStyle(GlassColor.textPrimary)
                            eyebrow(detail.measurementLabel)
                            mono(detail.witnessMeasurement)
                            eyebrow(detail.signerLabel)
                            mono(detail.signer)
                            caption(detail.verifiedAtReview)
                        }
                    }
                }
                .fixedSize(horizontal: false, vertical: true)
            } else if model.routeDisclosureState == .loading {
                GlassSpinner(standalone: true)
            } else {
                RouteDisclosureUnreadableGlassLine(line: model.routeDisclosureUnreadableCopy?.session)
            }
        }
        .onAppear {
            model.refreshRouteDisclosure()
            if entry.holdsCertificate { model.loadCertificateDetail(entryID: entry.entryID) }
        }
    }

    private func eyebrow(_ text: String) -> some View {
        Text(text)
            .glassType(GlassTokens.TypeScale.eyebrow)
            .foregroundStyle(GlassColor.textTertiary)
    }

    private func caption(_ text: String) -> some View {
        Text(text)
            .glassType(GlassTokens.TypeScale.caption)
            .foregroundStyle(GlassColor.textSecondary)
    }

    private func mono(_ text: String) -> some View {
        Text(text)
            .glassType(GlassTokens.TypeScale.mono)
            .foregroundStyle(GlassColor.textPrimary)
            .textSelection(.enabled)
    }
}
