import SwiftUI
import TCShellCore

/// K11: the raw send, both enclaves, and where the witness came from.
///
/// Every fact is the daemon's (`route_disclosure`) and every sentence is the
/// shared crate's (`tc_route_disclosure_copy`); this file only lays them out.
/// A block is drawn only when the Rust sent words for it, and
/// `RouteDisclosure.decode` has already refused words that do not match the
/// facts.
struct RouteDisclosureBody: View {
    let disclosure: RouteDisclosure

    var body: some View {
        let copy = disclosure.copy
        VStack(alignment: .leading, spacing: TC.Space.s) {
            Text(copy.route)
            if let line = copy.localFilter { Text(line) }
            if let facts = disclosure.facts.witness, let witness = copy.witness {
                VStack(alignment: .leading, spacing: TC.Space.xs) {
                    Text(witness.heading).font(TC.Font_.cardTitle)
                    TCFieldLabel(witness.addressLabel)
                    Text(facts.url).font(.system(.caption, design: .monospaced)).textSelection(.enabled)
                    TCFieldLabel(witness.signingLabel)
                    Text(facts.signingAddress).font(.system(.caption, design: .monospaced))
                        .textSelection(.enabled)
                    TCFieldLabel(witness.measurementsLabel)
                    ForEach(Array(facts.pinnedMeasurements.enumerated()), id: \.offset) { _, pin in
                        Text(pin).font(.system(.caption, design: .monospaced)).textSelection(.enabled)
                    }
                    Text(witness.check)
                    if let classifier = witness.classifier { Text(classifier) }
                    Text(witness.origin)
                }
                .padding(TC.Space.md)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(TC.surface)
            }
            if let line = copy.attestedBodies { Text(line) }
            if let line = copy.receipts { Text(line) }
        }
        .font(TC.Font_.caption)
        .fixedSize(horizontal: false, vertical: true)
    }
}

/// The Settings section. Unreadable is said as such and never drawn as some
/// other route; before the first answer it shows that it is loading.
struct RouteDisclosureSection: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.s) {
            switch model.routeDisclosureState {
            case .shown(let disclosure):
                TCSectionHeader(title: disclosure.copy.title)
                RouteDisclosureBody(disclosure: disclosure)
            case .loading:
                ProgressView().controlSize(.small)
            case .unreadable:
                if let title = model.routeDisclosureUnreadableCopy?.title {
                    TCSectionHeader(title: title)
                }
                RouteDisclosureUnreadableLine(line: model.routeDisclosureUnreadableCopy?.panel)
            }
        }
        .onAppear { model.refreshRouteDisclosure() }
    }
}

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
