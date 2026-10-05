import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The one rule the port field applies to what was typed. Out of range is
/// left as it was rather than clamped to something nobody typed: port 0 is
/// the ask-the-kernel sentinel, which the daemon refuses.
enum RoutingPortInput {
    static func accept(_ value: Int, into form: RoutingForm) -> RoutingForm {
        var next = form
        if let port = UInt16(exactly: value), port > 0 { next.port = port }
        return next
    }
}

/// What each tool does with the first hop out of this machine, and the
/// declaration that lets Trace Commons ask. Every string comes from the
/// core's routing copy; with no payload the card draws nothing, never
/// wording of its own, and never a healthy-looking row.
struct ToolsSection: View {
    @EnvironmentObject private var model: AppModel
    // The card's controls are the model's `routingDraft`, so a background
    // refresh landing mid-edit cannot replace a half-typed port and a
    // section switch cannot drop one (G8 of #1229). `nil` means nothing has
    // been edited and the card reads the daemon's answer, which is what
    // lets a port discovery supplies after appearing reach the field.
    /// Whether the override is open, once the contributor has said. `nil`
    /// follows discovery: closed where the machine supplied the port.
    @State private var routingOverrideOpen: Bool?

    var body: some View {
        // The container is always present, so `.onAppear` runs even when the
        // core's copy is missing and the card draws nothing.
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            if let copy = model.routingCopy {
                card(copy, form: model.routingDraft ?? model.routingForm)
            } else {
                // The unavailable branch; test_refreshIsOnAnAlwaysPresentContainer pins that
                // `.onAppear` follows this container's closing brace directly.
                Color.clear.frame(width: 0, height: 0).accessibilityHidden(true)
            }
        }
        .onAppear {
            // Asked every time the card appears: IronWire may have started
            // since. It reads a file, opens no connection, declares nothing.
            model.discoverRouting()
            model.refreshRoutedTools()
        }
    }

    private func card(_ copy: RoutingCopy, form: RoutingForm) -> some View {
        GlassEyebrowCard(copy.toolsHeading) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                // Only once the daemon has answered: `unset` is a real mode
                // to the core, so a default would word a tool the daemon may
                // have turned off. The loading state below covers the card.
                if let settings = model.daemonSettings {
                    ForEach(
                        RoutingSurface.toolRows(
                            sourceModes: settings.routingSourceModes,
                            evidence: model.routingEvidence,
                            copy: copy,
                            calls: model.routingCalls
                        ),
                        id: \.name
                    ) { row in
                        HStack {
                            Text(row.name)
                            Spacer()
                            // The tone rides on the row, decided by the same
                            // shared table that chose the word.
                            GlassTag(row.word, tone: Self.tone(row.tone))
                        }
                        .glassType(GlassTokens.TypeScale.body)
                        .accessibilityElement(children: .combine)
                        .accessibilityLabel("\(row.name): \(row.word)")
                    }
                }

                Text(copy.intro)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)

                Toggle(copy.toggle, isOn: Binding(
                    get: { form.on },
                    set: { on in
                        var next = form
                        next.on = on
                        model.routingDraft = next
                        model.applyIronWire(next)
                    }
                ))
                .toggleStyle(GlassToggleStyle(.settings))
                // With no settings the form reads off by default; a switch
                // drawn from that would read as a working "off".
                .disabled(model.daemonSettings == nil)

                if model.daemonSettings == nil {
                    SettingsReadNotice(model.settingsRead, retry: model.refreshSettings)
                } else {
                    routingState(copy)
                }

                Text(RoutingSurface.discoveryLine(
                    model.routingDiscovery, copy: copy, calls: model.routingCalls))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)

                HStack(spacing: GlassTokens.Space.s3) {
                    // The shortcut past the two fields, offered only where
                    // there is something to connect to and nothing declared.
                    if model.routingDiscovery.found, !form.on {
                        Button(copy.connect) {
                            let next = RoutingSurface.connecting(form)
                            model.routingDraft = next
                            model.applyIronWire(next)
                        }
                        .buttonStyle(GlassButtonStyle(.primary))
                        .disabled(model.routingChecking || model.daemonSettings == nil)
                    }
                    Button(copy.lookAgain) { model.discoverRouting() }
                        .buttonStyle(GlassButtonStyle(.link))
                }

                GlassExpander(copy.overrideTitle, isOpen: Binding(
                    get: {
                        routingOverrideOpen
                            ?? !RoutingSurface.overrideIsCollapsed(model.routingDiscovery)
                    },
                    set: { routingOverrideOpen = $0 }
                ))
                if routingOverrideOpen ?? !RoutingSurface.overrideIsCollapsed(model.routingDiscovery) {
                    override(copy, form: form)
                }

                Button(model.routingChecking ? copy.checking : copy.apply) {
                    model.applyIronWire(form)
                }
                .buttonStyle(GlassButtonStyle(.glass))
                .disabled(!form.on || model.routingChecking || model.daemonSettings == nil)

                if let probeLine = model.routingProbeLine {
                    Text(probeLine)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }

                Text(copy.appliesAtOnce)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    /// The port and folder, live only while the switch is on.
    private func override(_ copy: RoutingCopy, form: RoutingForm) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                Text(copy.portTitle)
                    .glassType(GlassTokens.TypeScale.eyebrow)
                    .foregroundStyle(GlassColor.textTertiary)
                TextField(
                    copy.portTitle,
                    value: Binding(
                        get: { Int(form.port) },
                        set: { model.routingDraft = RoutingPortInput.accept($0, into: form) }
                    ),
                    format: .number.grouping(.never)
                )
                .textFieldStyle(.plain)
                .glassType(GlassTokens.TypeScale.label.weight(.regular))
                .foregroundStyle(GlassColor.textPrimary)
                .padding(.horizontal, 10)
                .frame(maxWidth: 120, minHeight: GlassTokens.Size.controlLarge, alignment: .leading)
                .background(
                    RoundedRectangle(cornerRadius: GlassTokens.Radius.control, style: .continuous)
                        .fill(GlassTokens.Color.fieldFill.color)
                )
                .labelsHidden()
                .accessibilityLabel(copy.portTitle)
                Text(copy.portNote)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                Text(copy.folderTitle)
                    .glassType(GlassTokens.TypeScale.eyebrow)
                    .foregroundStyle(GlassColor.textTertiary)
                HStack(spacing: GlassTokens.Space.s3) {
                    GlassFolderButton(copy.chooseFolder) {
                        if let path = GlassSourceRow.chooseFolder() {
                            var next = form
                            next.tokenDir = path
                            model.routingDraft = next
                        }
                    }
                    .accessibilityLabel(copy.folderTitle)
                    Text(form.tokenDir)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .lineLimit(1)
                        .truncationMode(.head)
                }
                Text(copy.folderNote)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .disabled(!form.on)
    }

    /// The daemon's three-state view of what it is seeing, and when it last
    /// got an answer. `awaiting_rows` is held, never a fault.
    private func routingState(_ copy: RoutingCopy) -> some View {
        let state = model.status.routing.state
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            if model.status.routing.derived {
                Text(copy.derivedOrigin)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            // The tone is chosen from the state, never from the sentence.
            GlassStatusLabel(
                RoutingSurface.stateLine(state, copy: copy, calls: model.routingCalls),
                status: Self.status(RoutingSurface.tone(forState: state, calls: model.routingCalls)))
            // A stamp on the running daemon, shown only on a state that has
            // had an answer.
            if RoutingSurface.showsLastChecked(forState: state, calls: model.routingCalls),
               let at = model.status.routing.lastRefreshAt,
               let line = TCRoutingCopy.lastChecked(
                   when: Self.lastChecked.localizedString(for: at, relativeTo: Date())
               ) {
                Text(line)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
            }
        }
        .accessibilityElement(children: .combine)
    }

    /// Only the four routing tones exist; none reads as a failure.
    static func tone(_ tone: RoutingTone) -> GlassTag.Tone {
        switch tone {
        case .clear: return .on
        case .held: return .accent
        case .attention: return .ask
        case .neutral: return .neutral
        }
    }

    /// The same mapping for the status dot; the words carry the state.
    static func status(_ tone: RoutingTone) -> GlassStatus {
        switch tone {
        case .clear: return .on
        case .attention: return .ask
        case .held, .neutral: return .off
        }
    }

    /// A rendering of a timestamp, not wording about routing.
    private static let lastChecked: RelativeDateTimeFormatter = {
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .full
        return formatter
    }()
}
