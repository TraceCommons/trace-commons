import SwiftUI
import TCDesign

/// "How may your traces be used?" -- the onboarding consent-scope screen.
///
/// The most consequential screen in the product: a developer decides here
/// how real work transcripts, possibly an employer's or a client's, may be
/// used, and most people will give it about eight seconds. The copy below is
/// taken verbatim from the shared design spec
/// (`docs/superpowers/specs/2026-08-08-contributor-shell-shared-design.md`,
/// "### 3. Consent scopes"), not paraphrased.
///
/// The scope list itself -- name, description, `always_on`, `grants_data_use`
/// -- comes from the daemon's `consent_options` call (`AppModel.consentScopes`,
/// populated via `DaemonClient.consentOptions()`), never from a Swift literal
/// list. Three shells are being built against this contract; a hardcoded copy
/// here would drift from the protocol the moment the daemon adds or renames a
/// scope. The one Swift-side literal mapping that does exist,
/// `ScopeCopy.title(for:options:)`, is copy-only (the short bold label) and
/// is already shared with `PreviewSheet`/`ConsentSection` -- adding a second,
/// separate mapping in this file would be exactly the drift this rule warns
/// against, so it is reused rather than duplicated.
struct ConsentScopesView: View {
    @EnvironmentObject private var model: AppModel
    var onContinue: (Set<String>) -> Void
    /// Scopes the contributor had already ticked, when this screen is being
    /// re-entered from later in onboarding (screen 4 or 5) rather than seen
    /// for the first time -- see `OnboardingCoordinatorView`'s back
    /// navigation. Empty on first entry, matching the previous behavior.
    var initialSelection: Set<String> = []

    /// The one scroll for this step, in the wrapper as on the other steps,
    /// so every host scrolls exactly once.
    var body: some View {
        ScrollView {
            ConsentScopesContent(onContinue: onContinue, initialSelection: initialSelection)
                .environmentObject(model)
        }
    }
}

/// The screen's content, split out of its `ScrollView` for the same reason
/// `QueueContent` is split out of `QueueView`: `ImageRenderer` renders a
/// `ScrollView` as blank, and this is the highest-stakes screen in the app
/// to be leaving unverified.
struct ConsentScopesContent: View {
    @EnvironmentObject private var model: AppModel

    /// Names of optional (non-always-on) scopes the person has ticked.
    /// Nothing optional starts selected on first entry -- see rule 2 below
    /// -- but a re-entry from later in onboarding seeds this from whatever
    /// was chosen before, via `initialSelection`.
    @State private var selected: Set<String>

    var onContinue: (Set<String>) -> Void

    init(onContinue: @escaping (Set<String>) -> Void, initialSelection: Set<String> = []) {
        self.onContinue = onContinue
        _selected = State(initialValue: initialSelection)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
            header
            // Until the daemon has listed the scopes there is nothing to
            // choose and nothing to apply; the wait is drawn, never an
            // empty list read as "no permissions".
            if model.consentScopes.isEmpty {
                SettingsAwaiting()
            }
            groups
            Text(ConsentScopesWords.withdrawLater)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
            continueButton
        }
        .padding(GlassTokens.Space.panePadding)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            Text(ConsentScopesWords.heading)
                .glassType(GlassTokens.TypeScale.heading)
                .foregroundStyle(GlassColor.textPrimary)
            Text(ConsentScopesWords.changeLater)
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var groups: some View {
        // RULE 1: two visually distinct groups, because "always included"
        // and "optional" are two different kinds of decision -- one is a
        // fact about how the commons works, the other is a choice.
        let alwaysOn = model.consentScopes.filter(\.alwaysOn)
        // RULE 3: keyed off `grants_data_use`, not off the scope's name.
        // `public_attribution` is the one scope that grants no data use at
        // all (it only puts a handle on a list), so it is pulled into its
        // own "Credit" group rather than sitting beside real data-use scopes
        // -- next to them it would misread as a data permission, and it
        // would make the real ones look lighter than they are.
        let optional = model.consentScopes.filter { !$0.alwaysOn && $0.grantsDataUse }
        let credit = model.consentScopes.filter { !$0.alwaysOn && !$0.grantsDataUse }

        return VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
            if !alwaysOn.isEmpty {
                GlassEyebrowCard(ConsentScopesWords.alwaysIncluded) { rows(alwaysOn) }
            }
            if !optional.isEmpty {
                GlassEyebrowCard(ConsentScopesWords.optionalEachOne) { rows(optional) }
            }
            if !credit.isEmpty {
                GlassEyebrowCard(ConsentScopesWords.credit) { rows(credit) }
            }
        }
    }

    private func rows(_ scopes: [ConsentScope]) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            ForEach(scopes) { scope in
                scopeRow(scope)
            }
        }
    }

    @ViewBuilder
    private func scopeRow(_ scope: ConsentScope) -> some View {
        // RULE 2: always-on rows are locked on -- there is nothing to
        // toggle, they are included by definition. The rest follow the
        // local ticks, which start empty.
        if scope.alwaysOn {
            scopeToggle(scope, isOn: .constant(true))
        } else {
            scopeToggle(
                scope,
                isOn: Binding(
                    get: { ConsentScopeRows.isOn(scope: scope, granted: selected, unavailable: false) },
                    set: { granted in
                        if granted {
                            selected.insert(scope.name)
                        } else {
                            selected.remove(scope.name)
                        }
                    }))
        }
    }

    private func scopeToggle(_ scope: ConsentScope, isOn: Binding<Bool>) -> some View {
        Toggle(isOn: isOn) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                HStack(spacing: GlassTokens.Space.s2) {
                    Text(ScopeCopy.title(for: scope.name, options: model.consentScopes))
                        .glassType(GlassTokens.TypeScale.bodyStrong)
                    if scope.alwaysOn {
                        GlassTag(ConsentScopesWords.alwaysOn)
                    }
                }
                Text(scope.description)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .toggleStyle(GlassCheckboxStyle())
        .disabled(scope.alwaysOn)
        .accessibilityElement(children: .combine)
    }

    private var continueButton: some View {
        Button(
            UsesStep.continueLabel(
                alwaysOn: model.consentScopes.filter(\.alwaysOn).count, selected: selected.count)
        ) {
            onContinue(selected)
        }
        .buttonStyle(GlassButtonStyle(.primary))
        .keyboardShortcut(.defaultAction)
        .disabled(model.consentScopes.isEmpty)
    }
}

/// Pure rules for this step, kept apart from the view so they are testable.
enum UsesStep {
    /// The always-on scope(s) plus whatever optional or credit boxes are
    /// ticked, counted live -- not just the optional count, because the
    /// always-on permission is still a permission this upload carries.
    static func continueLabel(alwaysOn: Int, selected: Int) -> String {
        ConsentScopesWords.continueWith(alwaysOn + selected)
    }

    /// First entry ticks nothing optional.
    static func startsUnticked(_ initial: Set<String>) -> Bool { initial.isEmpty }
}

/// This screen's sentences, moved here unchanged from the view body.
enum ConsentScopesWords {
    static let heading = "How may your traces be used?"
    static let changeLater = "You can change this later. It applies to traces you send from now on."
    static let alwaysIncluded = "Always included"
    static let optionalEachOne = "Optional — each one lets your traces do more"
    static let credit = "Credit"
    static let withdrawLater = "To pull a trace back later, use History → Withdraw."
    static let alwaysOn = "always on"
    static func continueWith(_ total: Int) -> String {
        "Continue with \(total) \(total == 1 ? "permission" : "permissions")"
    }
}
