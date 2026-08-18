import SwiftUI

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
/// is already shared with `PreviewSheet`/`SettingsView` -- adding a second,
/// separate mapping in this file would be exactly the drift this rule warns
/// against, so it is reused rather than duplicated.
struct ConsentScopesView: View {
    @EnvironmentObject private var model: AppModel
    var onContinue: (Set<String>) -> Void = { _ in }
    /// Scopes the contributor had already ticked, when this screen is being
    /// re-entered from later in onboarding (screen 4 or 5) rather than seen
    /// for the first time -- see `OnboardingCoordinatorView`'s back
    /// navigation. Empty on first entry, matching the previous behavior.
    var initialSelection: Set<String> = []

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

    var onContinue: (Set<String>) -> Void = { _ in }

    init(onContinue: @escaping (Set<String>) -> Void = { _ in }, initialSelection: Set<String> = []) {
        self.onContinue = onContinue
        _selected = State(initialValue: initialSelection)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.xl) {
            header
            groups
            Text("To pull a trace back later, use History → Withdraw.")
                .font(TC.Font_.meta)
                .foregroundStyle(.secondary)
            continueButton
        }
        .padding(TC.Space.xxl)
        .tcColumn(TC.Measure.prose)
        .tcScreen()
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: TC.Space.xs) {
            Text("How may your traces be used?").font(TC.Font_.sectionTitle)
            Text("You can change this later. It applies to traces you send from now on.")
                .font(TC.Font_.meta)
                .foregroundStyle(.secondary)
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

        return VStack(alignment: .leading, spacing: TC.Space.xl) {
            if !alwaysOn.isEmpty {
                VStack(alignment: .leading, spacing: TC.Space.m) {
                    TCSectionHeader(title: "Always included")
                    ForEach(alwaysOn) { scope in
                        scopeRow(scope, checked: true, alwaysOn: true)
                    }
                }
            }
            if !optional.isEmpty {
                VStack(alignment: .leading, spacing: TC.Space.m) {
                    TCSectionHeader(title: "Optional — each one lets your traces do more")
                    ForEach(optional) { scope in
                        scopeRow(scope, checked: selected.contains(scope.name), alwaysOn: false)
                    }
                }
            }
            if !credit.isEmpty {
                VStack(alignment: .leading, spacing: TC.Space.m) {
                    TCSectionHeader(title: "Credit")
                    ForEach(credit) { scope in
                        scopeRow(scope, checked: selected.contains(scope.name), alwaysOn: false)
                    }
                }
            }
        }
    }

    private func scopeRow(_ scope: ConsentScope, checked: Bool, alwaysOn: Bool) -> some View {
        Button {
            // RULE 2: always-on rows are not interactive -- there is
            // nothing to toggle, they are included by definition.
            guard !alwaysOn else { return }
            if selected.contains(scope.name) {
                selected.remove(scope.name)
            } else {
                selected.insert(scope.name)
            }
        } label: {
            HStack(alignment: .top, spacing: TC.Space.m) {
                // A filled mark in the brand green when granted, an empty
                // box when not: the shape changes as well as the colour, so
                // "granted" is never carried by hue alone.
                TCReadGateCheckbox(checked: checked)
                VStack(alignment: .leading, spacing: TC.Space.xxs) {
                    HStack(spacing: TC.Space.s) {
                        Text(ScopeCopy.title(for: scope.name, options: model.consentScopes))
                            .font(TC.Font_.cardTitle)
                        if alwaysOn {
                            TCTag(text: "always on", tone: .clear, symbol: "lock")
                        }
                    }
                    Text(scope.description)
                        .font(TC.Font_.body)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 0)
            }
            .padding(TC.Space.m)
            .frame(maxWidth: .infinity, alignment: .leading)
            .tcCard()
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(alwaysOn)
        .accessibilityAddTraits(checked ? [.isSelected] : [])
    }

    private var continueButton: some View {
        // The always-on scope(s) plus whatever optional/credit boxes are
        // ticked, counted live -- not just the optional count, because the
        // always-on permission is still a permission this upload carries.
        let alwaysOnCount = model.consentScopes.filter(\.alwaysOn).count
        let total = alwaysOnCount + selected.count
        return Button("Continue with \(total) \(total == 1 ? "permission" : "permissions")") {
            onContinue(selected)
        }
        .tcPrimaryAction()
        .keyboardShortcut(.defaultAction)
    }
}
