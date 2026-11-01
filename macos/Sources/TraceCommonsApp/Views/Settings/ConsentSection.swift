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
        let alwaysOn = model.consentScopes.filter(\.alwaysOn)
        let optional = model.consentScopes.filter { !$0.alwaysOn && $0.grantsDataUse }

        GlassEyebrowCard(SettingsLegacyWords.consentEyebrow, title: SettingsLegacyWords.consentHeading, accessory: {
            GlassRefreshButton(SettingsLegacyWords.refresh, action: model.refreshStatus)
        }) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                caption(SettingsLegacyWords.appliesFromNow)
                // Until the daemon answers, the rows below are disabled and
                // read off (R-15); this says they are waiting, not refused.
                if model.statusRead != .answered {
                    SettingsReadNotice(model.statusRead, retry: model.refreshStatus)
                }
                ForEach(alwaysOn) { scope in
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                        HStack {
                            Text(scope.title).glassType(GlassTokens.TypeScale.bodyStrong)
                            GlassTag(SettingsLegacyWords.alwaysOn)
                        }
                        caption(scope.description)
                    }
                }
                caption(SettingsLegacyWords.rewardOpportunities)
                ForEach(optional) { scope in
                    ConsentScopeToggle(scope: scope)
                }
                // A refused write, unboxed, under the ticks it is about
                // (Ron, 2026-10-09).
                if model.consentWriteRefused {
                    GlassAlert(ConsentScopeRows.refusalLine(
                        settings: TCSourceChecks.settingsCopy()?.consentSaveFailed,
                        screens: ConsentScopeRows.screens))
                }
            }
        }
    }

    private func caption(_ sentence: String) -> some View {
        Text(sentence)
            .glassType(GlassTokens.TypeScale.caption)
            .foregroundStyle(GlassColor.textSecondary)
    }

}

/// One optional scope, saved from the daemon's reported list so editing
/// attribution cannot erase data-use permissions or change other choices.
struct ConsentScopeToggle: View {
    let scope: ConsentScope
    @EnvironmentObject private var model: AppModel

    var body: some View {
        let unavailable = !model.status.loggedIn || model.statusRead != .answered
        let isOn = Binding<Bool>(
            get: { ConsentScopeRows.isOn(scope: scope, granted: Set(model.status.consentScopes), unavailable: unavailable) },
            set: { granted in
                Task { await model.toggleConsentScope(scope, granted: granted, options: model.consentScopes) }
            })
        return Toggle(isOn: isOn) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                Text(scope.title).glassType(GlassTokens.TypeScale.bodyStrong)
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
}
