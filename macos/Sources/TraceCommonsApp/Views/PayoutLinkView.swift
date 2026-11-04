import SwiftUI
import TCDesign
import TCShellCore

/// The NEAR AI credit section: which organization the commons sends NEAR AI
/// credit to. Hidden unless the commons lists `near_ai` as enabled, or this
/// device has no account session, which the section says. Every
/// sentence is the core's draft copy (`PayoutLinkCopy`); this only lays them
/// out. The person's click on Choose, after the consent for the selected
/// organization is on screen, is the confirmation.
///
/// This standalone picker starts without a selection. First-run onboarding
/// configures a verified sole organization before showing this picker.
struct PayoutLinkView: View {
    @EnvironmentObject private var app: AppModel
    @StateObject private var model = PayoutLinkModel()

    var body: some View {
        // Always present, so `.task` runs even when the section is hidden.
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            if let surface = model.surface, let rail = model.rail {
                GlassEyebrowCard(surface.copy.title) {
                    content(surface, rail)
                }
            } else if let surface = model.surface, let refusal = model.statusRefusal {
                GlassEyebrowCard(surface.copy.title) {
                    if refusal == .sessionEnded {
                        // The session ended: the line is the action
                        // (owner: "It should just say Sign in again").
                        Button(surface.refusalLine(refusal)) {
                            Task {
                                await app.signInToAccount()
                                await model.refresh()
                            }
                        }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .disabled(app.accountSigningIn && !app.canRestartNearAiBrowserSignIn)
                    } else {
                        Text(surface.refusalLine(refusal))
                            .glassType(GlassTokens.TypeScale.body)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
        }
        .task(id: app.payoutLinkDaemonClient.map(ObjectIdentifier.init)) {
            model.attach(app.payoutLinkDaemonClient)
            await model.refresh()
        }
        .onChange(of: app.accountSessionGeneration) {
            Task { await model.refresh() }
        }
    }

    @ViewBuilder
    private func content(_ surface: PayoutLinkSurface, _ rail: PayoutRailStatus) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            if let line = surface.statusLine(for: rail) {
                Text(line)
                    .glassType(GlassTokens.TypeScale.body)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if surface.canUnlink(for: rail) {
                Text(surface.copy.unlinkBody)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                Button(surface.copy.unlinkButton) { Task { await model.stop() } }
                    .buttonStyle(GlassButtonStyle(.glass))
                    .accessibilityLabel(surface.unlinkAccessibilityName(for: rail) ?? surface.copy.unlinkButton)
                    .disabled(model.busy)
            }
            if surface.showsChangeButton(for: rail, model.picker) {
                Button(surface.copy.changeButton) { Task { await model.change() } }
                    .buttonStyle(GlassButtonStyle(.glass))
                    .accessibilityLabel(surface.copy.changeAccessibilityName)
                    .disabled(model.busy)
            }
            if surface.showsPicker(for: rail, model.picker) {
                picker(surface)
            }
            if let refusal = model.refusal {
                GlassNotice(tone: .outside) { Text(surface.refusalLine(refusal)) }
            }
        }
    }

    @ViewBuilder
    private func picker(_ surface: PayoutLinkSurface) -> some View {
        switch model.picker.phase {
        case .idle, .loading, .refused:
            EmptyView()
        case .noOrganization:
            Text(surface.copy.noOrganization)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
        case .choosing, .working:
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                ForEach(model.picker.rows) { org in
                    row(org)
                }
            }
            if let consent = model.picker.consent(copy: surface.copy) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    Text(consent.scope)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                    HStack {
                        Button(consent.button) { Task { await model.choose() } }
                            .buttonStyle(GlassButtonStyle(.glass))
                            .accessibilityLabel(consent.accessibilityName)
                            .disabled(model.busy)
                        Button(consent.secondary) { model.notNow() }
                            .buttonStyle(GlassButtonStyle(.link))
                            .disabled(model.busy)
                    }
                }
            }
        }
    }

    private func row(_ org: PayoutOrganization) -> some View {
        let selected = model.picker.selectedId == org.id
        return Button {
            model.select(org.id)
        } label: {
            HStack {
                Image(systemName: selected ? "largecircle.fill.circle" : "circle")
                    .accessibilityHidden(true)
                Text(org.name)
                Spacer(minLength: 0)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? [.isSelected] : [])
        .disabled(model.busy)
    }
}
