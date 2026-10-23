import SwiftUI
import TCDesign
import TCShellCore

/// Which NEAR AI organization Private AI spends from, in the Private AI
/// section of Settings (owner request, 2026-10-23).
///
/// Drawn only for an account with two or more active organizations: one
/// organization -- the common case -- draws nothing here, and a listing that
/// failed draws nothing either. Picking an organization switches to it; the
/// current one carries the menu's check. While a switch is in flight (about
/// ten seconds) the picker is held and the daemon's line says so; a refusal
/// is the daemon's sentence, and every refusal changed nothing.
///
/// Every word is the daemon's `view` (draft wording, awaiting owner review).
struct SpendOrganizationCard: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        let status = model.spendOrganizations
        if status.showsPicker, let words = status.words {
            GlassCard {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    HStack(alignment: .center, spacing: GlassTokens.Space.s4) {
                        Text(words.title)
                            .glassType(GlassTokens.TypeScale.bodyStrong)
                        Spacer(minLength: 0)
                        GlassSelect(
                            words.picker,
                            selection: Binding<String>(
                                get: { status.currentOrganizationID ?? "" },
                                set: { wanted in
                                    guard wanted != status.currentOrganizationID else { return }
                                    Task { await model.selectSpendOrganization(wanted) }
                                }),
                            options: Self.options(status))
                        .disabled(model.spendOrganizationSwitching || model.credentialBusy)
                    }
                    Text(words.what)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                    if let line = Self.stateLine(
                        status, switching: model.spendOrganizationSwitching,
                        failure: model.spendOrganizationFailure)
                    {
                        GlassNotice(tone: line.tone, title: line.text) { EmptyView() }
                    }
                }
            }
        }
    }

    /// The organizations by name, in the daemon's order.
    static func options(_ status: SpendOrganizations) -> [GlassPickerOption<String>] {
        status.organizations.map { GlassPickerOption($0.name, value: $0.id) }
    }

    struct Line: Equatable {
        let text: String
        let tone: GlassStatus
    }

    /// One line under the picker, at most: switching, else the last
    /// refusal, else a replaced key still being revoked.
    static func stateLine(_ status: SpendOrganizations, switching: Bool, failure: String?) -> Line? {
        guard let words = status.words else { return nil }
        if switching { return Line(text: words.switching, tone: .ask) }
        if let failure { return Line(text: failure, tone: .outside) }
        if status.revokePending { return Line(text: words.revokePending, tone: .ask) }
        return nil
    }
}
