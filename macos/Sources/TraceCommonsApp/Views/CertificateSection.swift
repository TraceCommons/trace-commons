import SwiftUI
import TCBridge
import TCShellCore

/// The sessions a witness certificate is held for, drawn together above the
/// folders they are scattered across.
///
/// **One fact, two readings.** Both witness routes produce a certificate, so
/// membership is one question -- `holdsCertificate` on the row. What differs
/// is the wording: a contributor with no invite is being told these are
/// candidates for submission, one with an invite that they are
/// cryptographically attested. Both sentences and the choice between them
/// come from the shared crate through `TCCertificate`; nothing here picks.
///
/// **Always drawn, heading and all, even with nothing in it.** A filtered
/// section that renders as nothing when nothing matches cannot be told apart
/// from one that failed to load. On this shell `holds_certificate` is
/// optional at the decoder, so a daemon that never sends it yields `false` on
/// every row and produces exactly that. The empty sentence is what says which
/// one a contributor is looking at.
struct CertificateSection: View {
    @EnvironmentObject private var model: AppModel
    let entries: [QueueEntry]

    /// `admission_evidence_required` verbatim, never negated. True for a
    /// contributor who signed up through NEAR and therefore has no invite,
    /// which is the candidate reading. Absent settings read as false, the
    /// reading that claims less.
    private var evidenceAdmitted: Bool {
        model.daemonSettings?.admissionEvidenceOffered == true
    }

    private var held: [QueueEntry] { entries.filter(\.holdsCertificate) }

    var body: some View {
        if let title = TCCertificate.listTitle(evidenceAdmitted: evidenceAdmitted) {
            VStack(alignment: .leading, spacing: TC.Space.s) {
                Text(title).font(TC.Font_.cardTitle)
                if held.isEmpty {
                    Text(model.privateInferenceCopy?.certificateListEmpty ?? "")
                        .font(TC.Font_.caption)
                        .foregroundStyle(TC.inkSecondary)
                } else if let line = TCCertificate.rowLine(evidenceAdmitted: evidenceAdmitted) {
                    ForEach(held) { entry in
                        VStack(alignment: .leading, spacing: TC.Space.micro) {
                            Text(entry.projectLabel)
                            Text(line)
                                .font(TC.Font_.caption)
                                .foregroundStyle(TC.inkSecondary)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
            }
            .padding(TC.Space.md)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(TC.surface)
        }
    }
}
