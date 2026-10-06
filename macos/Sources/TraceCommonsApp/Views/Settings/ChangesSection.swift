import SwiftUI
import TCBridge
import TCDesign

struct ChangesSection: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        GlassEyebrowCard(SettingsLegacyWords.auditHeading) {
            VStack(alignment: .leading, spacing: 0) {
                // The log defaults to empty, and a failed `list_audit` keeps
                // it so; until that call answers, "nothing changed" would be
                // a count nothing reported. `status` answering is not it.
                if model.auditRead != .answered {
                    SettingsReadNotice(model.auditRead, retry: model.refreshAudit)
                } else if model.audit.isEmpty {
                    Text(SettingsLegacyWords.nothingChanged)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                }
                // The rows carry no id on the wire, and two entries can
                // legally agree on every field they do carry, so the offset
                // in a newest-first list is the only stable identity.
                ForEach(Array(model.audit.enumerated()), id: \.offset) { index, entry in
                    GlassTableRow(first: index == 0) {
                        HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
                            Text(Self.instant(entry.at))
                                .glassType(GlassTokens.TypeScale.mono)
                                .foregroundStyle(GlassColor.textSecondary)
                            Text(SettingsLegacyWords.auditSentence(entry.action, project: entry.projectLabel))
                                .glassType(GlassTokens.TypeScale.body)
                                .fixedSize(horizontal: false, vertical: true)
                            Spacer(minLength: 0)
                        }
                        .accessibilityElement(children: .combine)
                    }
                }
            }
        }
        .onAppear { model.refreshAudit() }
    }

    /// Shown in the reader's own locale and time zone: the layer below has
    /// already decoded the instant to a `Date`.
    private static func instant(_ date: Date) -> String {
        date.formatted(.dateTime.month(.abbreviated).day().hour().minute())
    }
}
