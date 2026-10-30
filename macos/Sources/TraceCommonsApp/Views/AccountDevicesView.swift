import SwiftUI
import TCDesign
import TCShellCore

/// The Devices section: the computers and servers linked to this account,
/// Link a server (a one-time code for `login --code` on the server), and
/// Unlink for every device but this one. Hidden until the daemon answers,
/// except for an ended session, which it says. Every sentence is the core's
/// draft copy (`AccountDevicesCopy`); this only lays them out.
struct AccountDevicesView: View {
    @EnvironmentObject private var app: AppModel
    @StateObject private var model = AccountDevicesModel()

    var body: some View {
        // Always present, so `.task` runs even when the section is hidden.
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            if let surface = model.surface, let devices = model.devices {
                GlassEyebrowCard(surface.copy.title) {
                    content(surface, devices)
                }
            } else if let surface = model.surface, model.listRefusal == .sessionEnded {
                GlassEyebrowCard(surface.copy.title) {
                    Button(surface.copy.sessionEnded) {
                        Task {
                            await app.signInToAccount()
                            await model.refresh()
                        }
                    }
                    .buttonStyle(GlassButtonStyle(.glass))
                    .disabled(app.accountSigningIn && !app.canRestartNearAiBrowserSignIn)
                }
            }
        }
        .task(id: app.payoutLinkDaemonClient.map(ObjectIdentifier.init)) {
            model.attach(app.payoutLinkDaemonClient)
            await model.refresh()
            // The Traces map's + opened Settings here to link a server; the
            // model takes the request now that it has its client.
            await model.linkIfRequested()
        }
        // Asked again while the section is already on screen.
        .onReceive(AccountDeviceLinkRequest.shared.$pending.removeDuplicates().filter { $0 }) { _ in
            // Before `.task` has attached the client this leaves the
            // request for `.task` (`linkIfRequested`).
            Task { await model.linkIfRequested() }
        }
        .onChange(of: app.accountSessionGeneration) {
            Task { await model.refresh() }
        }
    }

    @ViewBuilder
    private func content(_ surface: AccountDevicesSurface, _ devices: [AccountDevice]) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            Text(surface.copy.body)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            ForEach(devices) { device in
                row(surface, device)
            }
            linking(surface)
            if let refusal = model.refusal {
                GlassNotice(tone: .outside) { Text(surface.refusalLine(refusal)) }
            }
        }
    }

    private func row(_ surface: AccountDevicesSurface, _ device: AccountDevice) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: GlassTokens.Space.s2) {
                        Text(surface.name(for: device))
                            .glassType(GlassTokens.TypeScale.body)
                        if device.thisDevice {
                            GlassChip(glass: surface.copy.thisDevice, muted: true)
                        }
                    }
                    if let seen = surface.lastSeenLine(for: device, formatted: Self.relative) {
                        Text(seen)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                    }
                }
                Spacer(minLength: 0)
                if surface.canUnlink(device), model.confirming != device {
                    Button(surface.copy.unlinkButton) { model.confirming = device }
                        .buttonStyle(GlassButtonStyle(.link))
                        .accessibilityLabel(surface.unlinkAccessibilityName(for: device))
                        .disabled(model.busy)
                }
            }
            if model.confirming == device {
                Text(surface.unlinkConfirm(for: device))
                    .glassType(GlassTokens.TypeScale.caption)
                    .fixedSize(horizontal: false, vertical: true)
                HStack {
                    Button(surface.copy.unlinkConfirmButton) { Task { await model.unlink(device) } }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .accessibilityLabel(surface.unlinkAccessibilityName(for: device))
                        .disabled(model.busy)
                    Button(surface.copy.cancelButton) { model.confirming = nil }
                        .buttonStyle(GlassButtonStyle(.link))
                }
            }
        }
    }

    @ViewBuilder
    private func linking(_ surface: AccountDevicesSurface) -> some View {
        switch model.link {
        case .idle, .minting:
            linkButton(surface)
        case .refused(let refusal):
            GlassNotice(tone: .outside) { Text(surface.refusalLine(refusal)) }
            linkButton(surface)
        case .waiting(let code):
            let command = surface.command(for: code)
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                Text(surface.copy.linkBody)
                    .glassType(GlassTokens.TypeScale.caption)
                    .fixedSize(horizontal: false, vertical: true)
                Text(command)
                    .glassType(GlassTokens.TypeScale.mono)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                HStack {
                    Button(model.copiedCode == code.codeId ? surface.copy.copied : surface.copy.copyButton) {
                        model.copyCommand(command, for: code)
                    }
                    .buttonStyle(GlassButtonStyle(.glass))
                    Button(surface.copy.doneButton) { model.dismissLink() }
                        .buttonStyle(GlassButtonStyle(.link))
                }
                HStack(spacing: GlassTokens.Space.s2) {
                    GlassSpinner()
                    Text(surface.copy.waiting)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                }
            }
        case .linked:
            HStack {
                Text(surface.copy.linked).glassType(GlassTokens.TypeScale.body)
                Button(surface.copy.doneButton) { model.dismissLink() }
                    .buttonStyle(GlassButtonStyle(.link))
            }
        case .expired:
            Text(surface.copy.expired)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
            linkButton(surface)
        }
    }

    private func linkButton(_ surface: AccountDevicesSurface) -> some View {
        Button(surface.copy.linkButton) { Task { await model.linkServer() } }
            .buttonStyle(GlassButtonStyle(.glass))
            .accessibilityLabel(surface.copy.linkAccessibilityName)
            .disabled(model.busy || model.link == .minting)
    }

    private static let parser: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter
    }()

    private static let relativeFormatter: RelativeDateTimeFormatter = {
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .full
        return formatter
    }()

    /// "5 minutes ago", or nil for a date this shell cannot read.
    static func relative(_ text: String) -> String? {
        let date = parser.date(from: text) ?? ISO8601DateFormatter().date(from: text)
        return date.map { relativeFormatter.localizedString(for: $0, relativeTo: Date()) }
    }
}
