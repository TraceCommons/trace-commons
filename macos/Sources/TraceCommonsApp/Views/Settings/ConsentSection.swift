import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The list `set_consent_scopes` is sent. Pure so the rule is testable: it
/// is built from what the daemon reports, never from the ticks on screen.
enum ConsentScopeRows {
    static func nextScopes(
        reported: [String], options: [ConsentScope], toggling scope: ConsentScope, granted: Bool
    ) -> Set<String> {
        var scopes = Set(reported)
        scopes.formUnion(options.filter(\.alwaysOn).map(\.name))
        if granted {
            scopes.insert(scope.name)
        } else if !scope.alwaysOn {
            scopes.remove(scope.name)
        }
        return scopes
    }

    /// With no daemon answer nothing reads as granted, the always-on row
    /// included; otherwise always-on is locked on and the rest follow the
    /// daemon's list.
    static func isOn(scope: ConsentScope, granted: Set<String>, unavailable: Bool) -> Bool {
        !unavailable && (scope.alwaysOn || granted.contains(scope.name))
    }

    static func isEnabled(scope: ConsentScope, busy: Bool, unavailable: Bool) -> Bool {
        !scope.alwaysOn && !busy && !unavailable
    }

    /// The core's monitor screens table, decoded once (as `MonitorWords`).
    static let screens = MonitorScreensCopy.decode(fromJSON: TCCoreCopy.monitorScreensCopyJSON())

    /// The line a refused consent write draws, all of it the core's: the
    /// settings table's sentence, else the monitor table's request-failed
    /// sentence, else the dash every surface uses for "the core said
    /// nothing". A refusal never draws nothing.
    static func refusalLine(settings: String?, screens: MonitorScreensCopy?) -> String {
        settings ?? screens?.requestFailed ?? "\u{2014}"
    }
}

struct ConsentSection: View {
    /// The write's in-flight flag and refusal live on the model, not here:
    /// this view is thrown away when the section changes (G8 of #1229).
    @EnvironmentObject private var model: AppModel

    var body: some View {
        let granted = Set(model.status.consentScopes)
        let alwaysOn = model.consentScopes.filter(\.alwaysOn)
        let optional = model.consentScopes.filter { !$0.alwaysOn && $0.grantsDataUse }
        // Scopes that grant no data use: keyed off the daemon's
        // `grants_data_use`, never a name. Kept apart from the real ones.
        let credit = model.consentScopes.filter { !$0.alwaysOn && !$0.grantsDataUse }

        GlassEyebrowCard(SettingsLegacyWords.consentHeading) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                caption(SettingsLegacyWords.appliesFromNow)
                // Until the daemon answers, the rows below are disabled and
                // read off (R-15); this says they are waiting, not refused.
                if model.statusRead != .answered {
                    SettingsReadNotice(model.statusRead, retry: model.refreshStatus)
                }
                group(SettingsLegacyWords.alwaysIncluded, alwaysOn, granted: granted)
                group(SettingsLegacyWords.optionalEachOne, optional, granted: granted)
                group(SettingsLegacyWords.credit, credit, granted: granted)
                if model.consentWriteRefused {
                    GlassNotice(tone: .outside) {
                        Text(ConsentScopeRows.refusalLine(
                            settings: TCSourceChecks.settingsCopy()?.consentSaveFailed,
                            screens: ConsentScopeRows.screens))
                    }
                }
                caption(SettingsLegacyWords.nothingPreselected)
            }
        }
    }

    private func caption(_ sentence: String) -> some View {
        Text(sentence)
            .glassType(GlassTokens.TypeScale.caption)
            .foregroundStyle(GlassColor.textSecondary)
    }

    @ViewBuilder
    private func group(_ title: String, _ scopes: [ConsentScope], granted: Set<String>) -> some View {
        if !scopes.isEmpty {
            Text(title)
                .glassType(GlassTokens.TypeScale.eyebrow)
                .foregroundStyle(GlassColor.textSecondary)
            ForEach(scopes) { scope in
                row(scope, granted: granted)
            }
        }
    }

    /// The tick reads `status.consentScopes` -- what the daemon holds -- never
    /// a draft, so nothing optional shows as granted that the daemon does
    /// not hold. With no daemon answer (`!model.status.loggedIn`) the row is
    /// disabled and reads off.
    private func row(_ scope: ConsentScope, granted: Set<String>) -> some View {
        let unavailable = !model.status.loggedIn
        let isOn = Binding<Bool>(
            get: { ConsentScopeRows.isOn(scope: scope, granted: granted, unavailable: unavailable) },
            set: { setScope(scope, granted: $0) })
        return Toggle(isOn: isOn) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                HStack(spacing: GlassTokens.Space.s2) {
                    Text(ScopeCopy.title(for: scope.name, options: model.consentScopes))
                        .glassType(GlassTokens.TypeScale.bodyStrong)
                    if scope.alwaysOn {
                        GlassTag(SettingsLegacyWords.alwaysOn)
                    }
                }
                Text(scope.description)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .toggleStyle(GlassCheckboxStyle())
        .disabled(!ConsentScopeRows.isEnabled(scope: scope, busy: model.consentWriteBusy, unavailable: unavailable))
        .accessibilityElement(children: .combine)
    }

    /// Adds or removes one optional scope through the model, which builds
    /// the list from the daemon's own and holds the write's state.
    private func setScope(_ scope: ConsentScope, granted: Bool) {
        Task { await model.toggleConsentScope(scope, granted: granted, options: model.consentScopes) }
    }
}
