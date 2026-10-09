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
    /// The offer `place` draws (`NudgeSettings.offers(_:copy:on:)`).
    func offers(on place: NudgeSurface.Place) -> [NudgeSettings.Offer] {
        NudgeSettings.offers(settings, copy: copy, on: place)
    }
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
    /// Answers whether the daemon took the write.
    @discardableResult
    func answer(_ offer: NudgeSettings.Offer, accept: Bool) async -> Bool {
        await write { client in
            if accept {
                try await client.setNotifyKind(offer.kind, on: true)
            } else {
                try await client.dismissNotifyOffer(kind: offer.kind)
            }
        }
    }

    /// Turn on, then `prompt` -- the system's permission prompt -- only if
    /// the daemon took the write: a refused one leaves the kind off, and a
    /// prompt for it would ask permission for nothing. Answers whether the
    /// write was taken.
    @discardableResult
    func accept(_ offer: NudgeSettings.Offer, then prompt: () async -> Void) async -> Bool {
        guard await answer(offer, accept: true) else { return false }
        await prompt()
        return true
    }

    /// Answers whether the write was sent and taken.
    @discardableResult
    private func write(_ body: (any DaemonDataClient) async throws -> Void) async -> Bool {
        guard !writing else { return false }
        writing = true
        defer { writing = false }
        writeError = nil
        guard let client else {
            writeError = .unreachable
            return false
        }
        do {
            try await body(client)
        } catch {
            writeError = error as? DaemonDataError ?? .undecodable(method: "set_settings")
            return false
        }
        await load()
        return true
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
                NudgeOfferCard(offer: offer, store: store, requestAuthorization: requestAuthorization)
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

/// One one-time offer: the core's sentence and its two answers. Turn on
/// asks for the system's permission afterwards, only once the daemon took
/// the write.
struct NudgeOfferCard: View {
    let offer: NudgeSettings.Offer
    let store: NudgeSettingsStore
    var requestAuthorization: () async -> Void

    var body: some View {
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
                        Task { await store.accept(offer) { await requestAuthorization() } }
                    }
                    .buttonStyle(GlassButtonStyle(.glass))
                }
                .disabled(store.writing)
            }
        }
    }
}

/// The one-time offer for a page's own kind -- the verdicts offer on
/// History, the idle one on Traces -- for an install that existed before
/// the two kinds. The same settings, words and answers as Settings; once
/// answered either way the daemon clears it and it is not drawn again. A
/// refused answer is said under it, in the core's line.
struct NudgeOfferCards: View {
    let place: NudgeSurface.Place
    @EnvironmentObject private var model: AppModel
    @State private var store = NudgeSettingsStore(client: nil)

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            ForEach(Array(store.offers(on: place).enumerated()), id: \.offset) { _, offer in
                NudgeOfferCard(offer: offer, store: store) {
                    _ = await Notifier.shared.requestAuthorizationIfNeverAsked()
                }
            }
            if let error = store.writeError, let line = MonitorWords.table?.line(for: error) {
                GlassAlert(line)
            }
        }
        .task(id: model.liveData.map(ObjectIdentifier.init)) {
            store.attach(model.daemonData)
            await store.load()
        }
    }
}
