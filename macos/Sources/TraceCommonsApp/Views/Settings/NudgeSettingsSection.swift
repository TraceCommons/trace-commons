import Observation
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The suggestion and notification switches (nudge A2), read and written
/// through `DaemonDataClient`. Every word is the core's
/// (`tc_nudge_copy_json`); a switch is drawn only for a value the daemon
/// reported.
@MainActor
@Observable
final class NudgeSettingsStore {
    private(set) var settings: DaemonData.Settings?
    /// The last read that failed; nil once one succeeds.
    private(set) var readError: DaemonDataError?
    /// The last write the core refused, until the next write.
    private(set) var writeError: DaemonDataError?
    /// A write in flight.
    private(set) var writing = false
    private(set) var client: (any DaemonDataClient)?
    /// The core's fixed nudge words, decoded once.
    let copy: NudgeCopy? = NudgeCopy.decode(fromJSON: TCCoreCopy.nudgeCopyJSON())

    init(client: (any DaemonDataClient)?) {
        self.client = client
    }

    /// Follows a new client: nothing the old one said is drawn.
    func attach(_ client: (any DaemonDataClient)?) {
        self.client = client
        settings = nil
        readError = nil
        writeError = nil
        writing = false
    }

    var rows: [NudgeSettings.Row] { NudgeSettings.rows(settings, copy: copy) }
    var offers: [NudgeSettings.Offer] { NudgeSettings.offers(settings, copy: copy) }
    var footnote: String? { rows.isEmpty ? nil : NudgeSettings.footnote(copy: copy) }

    func load() async {
        guard let client else {
            settings = nil
            readError = .unreachable
            return
        }
        do {
            settings = try await client.settings()
            readError = nil
        } catch {
            // Unknown, never off: no switch is drawn from a failed read.
            settings = nil
            readError = error as? DaemonDataError ?? .undecodable(method: "get_settings")
        }
    }

    /// One switch, then a fresh read: the switch shows what the daemon
    /// stored, never a guess.
    func set(_ id: NudgeSettings.Switch, on: Bool) async {
        await write { try await NudgeSettings.write(id, on: on, through: $0) }
    }

    /// A one-time offer. Turn on writes the kind on, which also ends the
    /// offer; No thanks clears the offer's marker and changes nothing else.
    func answer(_ offer: NudgeSettings.Offer, accept: Bool) async {
        await write { client in
            if accept {
                try await client.setNotifyKind(offer.kind, on: true)
            } else {
                try await client.dismissNotifyOffer(kind: offer.kind)
            }
        }
    }

    private func write(_ body: (any DaemonDataClient) async throws -> Void) async {
        guard !writing else { return }
        writing = true
        defer { writing = false }
        writeError = nil
        guard let client else {
            writeError = .unreachable
            return
        }
        do {
            try await body(client)
        } catch {
            writeError = error as? DaemonDataError ?? .undecodable(method: "set_settings")
            return
        }
        await load()
    }
}

/// The switches and offers, inside the Notifications card: the suggestions
/// switch and the mark under it, the master switch and each kind under it,
/// then the core's line on the caps. The held kinds are never drawn.
struct NudgeSettingsSection: View {
    @EnvironmentObject private var model: AppModel
    @State private var store = NudgeSettingsStore(client: nil)
    /// Asks for the system's permission after an offer is accepted, when it
    /// was never asked: a kind turned on that can never post would be a
    /// switch that does nothing.
    var requestAuthorization: () async -> Void = {}

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            ForEach(Array(store.offers.enumerated()), id: \.offset) { _, offer in
                GlassCard(quiet: true) {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                        Text(offer.text)
                            .glassType(GlassTokens.TypeScale.body)
                            .foregroundStyle(GlassColor.textPrimary)
                            .fixedSize(horizontal: false, vertical: true)
                        HStack(spacing: GlassTokens.Space.s3) {
                            Button(offer.decline) { Task { await store.answer(offer, accept: false) } }
                                .buttonStyle(GlassButtonStyle(.glass))
                            Button(offer.accept) {
                                Task {
                                    await store.answer(offer, accept: true)
                                    await requestAuthorization()
                                }
                            }
                            .buttonStyle(GlassButtonStyle(.glass))
                        }
                        .disabled(store.writing)
                    }
                }
            }
            ForEach(store.rows, id: \.id) { row in
                VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                    Toggle(row.label, isOn: Binding(
                        get: { row.isOn },
                        set: { on in Task { await store.set(row.id, on: on) } }))
                        .toggleStyle(GlassToggleStyle(.settings))
                        .disabled(!row.enabled || store.writing)
                    if let help = row.help {
                        Text(help)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .padding(.leading, Self.indent(row.id))
            }
            if let footnote = store.footnote {
                Text(footnote)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let error = store.writeError ?? store.readError, let line = MonitorWords.table?.line(for: error) {
                GlassAlert(line)
            }
        }
        .task(id: model.liveData.map(ObjectIdentifier.init)) {
            store.attach(model.daemonData)
            await store.load()
        }
    }

    /// A finer switch sits under the broader one it follows.
    static func indent(_ id: NudgeSettings.Switch) -> CGFloat {
        switch id {
        case .suggestions, .notifications: 0
        case .menuBarMark, .notify: GlassTokens.Space.s6
        }
    }
}
