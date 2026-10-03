import SwiftUI
import TCBridge
import TCDesign
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
            GlassCard(quiet: true) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    Text(title)
                        .glassType(GlassTokens.TypeScale.bodyStrong)
                        .foregroundStyle(GlassColor.textPrimary)
                    if held.isEmpty {
                        Text(model.privateInferenceCopy?.certificateListEmpty ?? "")
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                    } else if let line = TCCertificate.rowLine(evidenceAdmitted: evidenceAdmitted) {
                        VStack(spacing: 0) {
                            ForEach(Array(held.enumerated()), id: \.element.id) { index, entry in
                                GlassTableRow(first: index == 0) {
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text(entry.projectLabel)
                                            .foregroundStyle(GlassColor.textPrimary)
                                        Text(line)
                                            .glassType(GlassTokens.TypeScale.caption)
                                            .foregroundStyle(GlassColor.textSecondary)
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
