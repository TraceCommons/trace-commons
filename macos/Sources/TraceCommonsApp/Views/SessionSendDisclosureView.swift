import SwiftUI
import TCShellCore

// The review sheet's per-session send disclosure, moved unchanged from the
// retired `RouteDisclosureView.swift` when Settings' route-disclosure panel
// moved to glass (`Views/Settings/PrivateAISection.swift`). The unreadable
// line moved with it because this view draws it.

/// The unreadable state. Marked by a glyph as well as colour, so it
/// survives greyscale and colour-blindness; and drawn even when the Rust's
/// sentence for it could not be read, so the panel is never simply empty.
struct RouteDisclosureUnreadableLine: View {
    let line: String?

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: TC.Space.s) {
            Image(systemName: TC.Tone.attention.symbol)
                .imageScale(.small)
                .foregroundStyle(TC.Tone.attention.color)
                .accessibilityHidden(line != nil)
            if let line {
                Text(line)
                    .font(TC.Font_.caption)
                    .foregroundStyle(TC.Tone.attention.textColor)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .accessibilityElement(children: .combine)
    }
}

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
        Group {
            if let disclosure = model.routeDisclosure {
                let session = disclosure.copy.session
                VStack(alignment: .leading, spacing: TC.Space.xs) {
                    TCFieldLabel(session.heading)
                    Text("\(session.beforeLabel) · \(Format.bytes(rawSessionBytes))").bold()
                    Text(session.beforeLine)
                    if let line = disclosure.copy.localFilter { Text(line) }
                    Text("\(session.afterLabel) · \(Format.bytes(wouldSendBytes))").bold()
                    Text(session.afterLine)
                    if disclosure.sendsToWitness { Text(disclosure.copy.route) }
                    if let detail = model.certificateDetails[entry.entryID] {
                        VStack(alignment: .leading, spacing: TC.Space.micro) {
                            Text(detail.heading).bold()
                            TCFieldLabel(detail.measurementLabel)
                            Text(detail.witnessMeasurement)
                                .font(.system(.caption, design: .monospaced))
                            TCFieldLabel(detail.signerLabel)
                            Text(detail.signer).font(.system(.caption, design: .monospaced))
                            Text(detail.verifiedAtReview)
                        }
                    }
                }
                .font(TC.Font_.caption)
                .fixedSize(horizontal: false, vertical: true)
            } else if model.routeDisclosureState == .loading {
                ProgressView().controlSize(.small)
            } else {
                RouteDisclosureUnreadableLine(line: model.routeDisclosureUnreadableCopy?.session)
            }
        }
        .onAppear {
            model.refreshRouteDisclosure()
            if entry.holdsCertificate { model.loadCertificateDetail(entryID: entry.entryID) }
        }
    }
}
