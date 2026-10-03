import SwiftUI
import TCBridge
import TCDesign

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
}

struct ConsentSection: View {
    @EnvironmentObject private var model: AppModel
    @State private var saveError: String?
    @State private var busy = false

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
                group(SettingsLegacyWords.alwaysIncluded, alwaysOn, granted: granted)
                group(SettingsLegacyWords.optionalEachOne, optional, granted: granted)
                group(SettingsLegacyWords.credit, credit, granted: granted)
                if let saveError {
                    GlassNotice(tone: .outside) { Text(saveError) }
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
        .disabled(!ConsentScopeRows.isEnabled(scope: scope, busy: busy, unavailable: unavailable))
        .accessibilityElement(children: .combine)
    }

    /// Adds or removes one optional scope, sending the daemon's own list with
    /// this one changed, so two quick presses cannot drop a scope neither
    /// touched.
    private func setScope(_ scope: ConsentScope, granted: Bool) {
        guard !busy, model.status.loggedIn, !scope.alwaysOn else { return }
        let scopes = ConsentScopeRows.nextScopes(
            reported: model.status.consentScopes, options: model.consentScopes, toggling: scope, granted: granted)
        saveError = nil
        busy = true
        Task {
            if case .failed = await model.setConsentScopes(Array(scopes)) {
                saveError = TCSourceChecks.settingsCopy()?.consentSaveFailed
            }
            busy = false
        }
    }
}
