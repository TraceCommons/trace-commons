//! The dropdown.
//
// One pane. A backend is a thing you turn on and then watch, so the switch that
// turns it on and the capacity it reports live on the same row: off before
// consent, showing what the backend reports after it. Splitting those into a
// Status tab and a Settings tab meant the answer to "why is nothing routing
// here" was behind a segmented control.
//
// This mirrors `render::status()` in `src/render.rs`, which is the reference for
// what is honest to show — and, more usefully, for what not to. Three of its
// rules are load-bearing here:
//
// - **No bar without an observation.** `Format.capacityFraction` returns `nil`
//   for everything but `observed`, and this file draws nothing when it does.
// - **The privacy line is verbatim.** No shield, no lock, no "protected".
//   `docs/TRUST.md` I7 forbids describing the filter by what the user is safe
//   from, and an icon is a description.
// - **An update is news, never a button.** `docs/UPDATES.md` §1: the daemon
//   holds credentials in the middle of streamed responses and never updates
//   itself. A menu bar app is the most tempting place to break that.
//
// The fourth rule is the menu bar's own, and it is why this file keeps
// shrinking: **a control says its own state, so nothing under it repeats that.**
// A switch that is off already reads "off"; a caption saying so is a second
// voice for the same fact, and a dozen of them is what made this pane a wall of
// orange text. What a control cannot say — the consent question, the file a
// tool's row would write — is carried as its tooltip and its VoiceOver hint,
// and a list nobody is reading right now lives behind a submenu. Text is left
// for the things with no control at all: a credential that is missing, a
// warning about what a write did not do.

import AppKit
import IronWireKit
import SwiftUI

struct MenuContent: View {
    @ObservedObject var client: ControlClient
    @ObservedObject var notifier: Notifier
    @ObservedObject var loginItem: LoginItem
    @State private var pinError: String?
    @State private var settingsError: String?
    @State private var settingsWarning: String?
    @State private var busy = false
    /// Whether the control URL was just put on the clipboard. A copy with no
    /// acknowledgement leaves the user pressing it twice.
    @State private var urlCopied = false
    /// Whether the one-time offer to wire everything has been answered.
    /// Persisted: asking again every launch is how a prompt stops being read.
    @AppStorage("toolsOnboardingAnswered") private var onboardingAnswered = false

    private var iconState: IconState { IconState.from(client.status) }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            Divider()
            if let status = client.status {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 14) {
                        onboarding()
                        backendSection(status)
                        toolsSection()
                        usageSection(status.usage)
                        balanceSection(status.balance)
                        routeSection(status)
                        pinSection(status)
                        privacySection(status)
                        updateSection(status.update)
                        if let settingsWarning {
                            Text(settingsWarning).font(.caption).foregroundStyle(.orange)
                        }
                        if let settingsError {
                            Text(settingsError).font(.caption).foregroundStyle(.red)
                        }
                    }
                    .padding(12)
                }
                .frame(maxHeight: 420)
            } else {
                notRunning
            }
            Divider()
            footer
        }
        .frame(width: 340)
        // The only place the poll rate changes. `/_ironwire/status` re-reads a
        // credential per backend, so one second is for when someone is actually
        // watching and five is for the rest of the time.
        .onAppear {
            client.menuIsOpen = true
            // The system is the authority on this, and the user can change it in
            // System Settings while we are not looking.
            loginItem.refresh()
            // The consent prompts and the privacy ladder live here, and this
            // pane cannot offer either without them.
            Task { await client.refreshSettings() }
        }
        .onDisappear { client.menuIsOpen = false }
    }

    // MARK: - Header

    private var header: some View {
        HStack(spacing: 8) {
            Circle()
                .fill(headerColour)
                .frame(width: 8, height: 8)
            VStack(alignment: .leading, spacing: 1) {
                Text(client.status.map { "IronWire \($0.version)" } ?? "IronWire")
                    .font(.headline)
                Text(headerDetail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
        }
        .padding(12)
    }

    private var headerColour: Color {
        switch iconState {
        case .healthy: return .green
        case .degraded: return .orange
        case .attention: return .red
        case .unreachable: return .secondary
        }
    }

    private var headerDetail: String {
        switch client.connection {
        case .unauthorised:
            return "the control token was rejected"
        case .connecting, .connected, .unreachable:
            return iconState.summary
        }
    }

    // MARK: - Daemon absent

    /// Not an error, and never a dialog. A daemon that is not running is the
    /// state a machine is in most of the time, and this app recovers from it on
    /// its own when one appears.
    private var notRunning: some View {
        VStack(alignment: .leading, spacing: 10) {
            if client.connection == .unauthorised {
                Text("IronWire is running, but rejected this app's token.")
                    .font(.callout)
                Text("The token is read from \(Discovery.home().path)/control.token. Restarting the daemon rewrites it.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                Text("IronWire is not running.")
                    .font(.callout)
                if let binary = Discovery.daemonBinary() {
                    Button("Start IronWire") { start(binary) }
                    Text(binary.path)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                } else {
                    Text("No `ironwire` binary was found. Install it, then run `ironwire serve`.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(12)
    }

    /// Best-effort, and nothing more. Starting a daemon is the user's decision
    /// and the CLI's job; this only saves them a trip to a terminal.
    private func start(_ binary: URL) {
        let process = Process()
        process.executableURL = binary
        process.arguments = ["serve"]
        // Nothing reads these, and an unread pipe that fills would wedge the
        // daemon rather than the app.
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        try? process.run()
    }

    // MARK: - Backends

    /// The settings-side record for a backend, when it has one. The two
    /// documents are joined here rather than in the daemon because they have
    /// different refresh rates: status polls, settings is fetched on open.
    private func service(for id: String) -> ServiceView? {
        client.servicesById[id]
    }

    @ViewBuilder
    private func backendSection(_ status: StatusView) -> some View {
        if status.backends.isEmpty {
            section("Backends") {
                Text("None configured. Run `ironwire connect claude`.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        } else {
            section("Backends") {
                ForEach(status.backends) { backend in
                    backendRow(backend)
                }
            }
        }
    }

    private func backendRow(_ backend: BackendView) -> some View {
        let service = service(for: backend.id)
        let prompt = service?.consentPrompt
        let switchable = service?.canToggle == true && prompt?.isComplete == true
            && (service?.consented == true || prompt?.binding?.isEmpty == false)
        // The same daemon field in both documents. The row's own copy comes with
        // every poll, so it is there before settings has been fetched; the
        // settings copy covers a daemon from before status carried it.
        let connectCommand = backend.connectCommand ?? service?.connectCommand

        return VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 6) {
                Text(backend.name).font(.callout.weight(.medium))
                Spacer()
                if switchable, let service, let prompt {
                    consentSwitch(service, prompt: prompt)
                } else {
                    Text(Format.kind(backend.kind))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }

            if !backend.authenticated {
                // A credential this app cannot conjure. It comes from the user
                // logging into Claude Code or Codex, or exporting an API key, so
                // the row names that command rather than offering a login.
                Text(backend.detail ?? "not connected")
                    .font(.caption)
                    .foregroundStyle(.red)
                if let command = connectCommand {
                    Text(command)
                        .font(.caption2.monospaced())
                        .textSelection(.enabled)
                        .foregroundStyle(.secondary)
                }
            } else if backend.consented {
                capacity(backend.headroom)
                if let health = Format.healthSummary(backend.health) {
                    Text(health)
                        .font(.caption)
                        .foregroundStyle(backend.health.isOpen ? .red : .orange)
                }
            } else if !switchable {
                // The one unconsented row that still needs words: there is no
                // switch here to say "off" for it, because the prompt arrived
                // incomplete or the credential is not usable.
                Text("not enabled")
                    .font(.caption)
                    .foregroundStyle(.orange)
                // The daemon's own field, verbatim. Composing one here —
                // appending a flag, substituting a backend name — is how the old
                // pane came to tell every backend to run `ironwire connect
                // claude`.
                if let command = connectCommand {
                    Text(command)
                        .font(.caption2.monospaced())
                        .textSelection(.enabled)
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

    /// The switch that grants or revokes consent.
    ///
    /// One flip is the whole action. The version travels with it — the one that
    /// was on screen, not the newest this build knows about, because an answer
    /// belongs to the question it answered.
    private func consentSwitch(_ service: ServiceView, prompt: ConsentPromptView) -> some View {
        Toggle("", isOn: Binding(
            get: { service.consented },
            set: { wanted in
                setConsent(service, granted: wanted, version: wanted ? prompt.version : 0, binding: prompt.binding)
            }
        ))
        .labelsHidden()
        .toggleStyle(.switch)
        .controlSize(.mini)
        .disabled(busy)
        .help(consentText(prompt))
        .accessibilityLabel(Text("Enable \(service.name)"))
        .accessibilityHint(Text(consentText(prompt)))
    }

    /// Everything the daemon has to say about a subscription, in its order and
    /// unabridged, as one string.
    ///
    /// Carried rather than drawn: it is the switch's tooltip and its VoiceOver
    /// hint, and it appears nowhere on the row. Paragraphs per backend is what
    /// made this pane unreadable, and a switch that is off already says it is
    /// off. `docs/TRUST.md` §2 records what that trade costs — the CLI is the
    /// gate that renders the whole question and waits for an answer.
    private func consentText(_ prompt: ConsentPromptView) -> String {
        ([prompt.summary] + prompt.points.map { "· \($0)" }).joined(separator: "\n")
    }

    private func setConsent(_ service: ServiceView, granted: Bool, version: Int, binding: String?) {
        busy = true
        Task {
            settingsError = nil
            let outcome = await client.setConsent(
                backend: service.id, granted: granted, promptVersion: version, binding: binding)
            if case .failure(let failure) = outcome {
                settingsError = failure.message
            }
            busy = false
        }
    }

    /// The bar, or the absence of one.
    ///
    /// `capacityFraction` is `nil` for every state but `observed`, and this is
    /// the branch that honours it. A `ProgressView` needs a `Double` and there
    /// is no honest one to give it here — 0 reads as empty, a grey half-bar
    /// reads as half, and both are numbers nobody measured.
    @ViewBuilder
    private func capacity(_ headroom: HeadroomView) -> some View {
        if let fraction = Format.capacityFraction(headroom) {
            ProgressView(value: fraction)
                .progressViewStyle(.linear)
                .tint(colour(for: Format.usageLevel(usedPct: fraction * 100)))
        }
        Text(Format.headroomSummary(headroom))
            .font(.caption)
            .foregroundStyle(.secondary)
    }

    private func colour(for level: Format.UsageLevel) -> Color {
        switch level {
        case .good: return .green
        case .warn: return .orange
        case .bad: return .red
        }
    }

    // MARK: - Onboarding

    /// Offer, once, to point every agent on this machine at IronWire.
    ///
    /// A daemon nothing routes through is the state a fresh install is in, and
    /// the one a user is least equipped to diagnose: every backend healthy,
    /// capacity plentiful, and their agent talking straight to the provider.
    /// Asking once is the whole of the interaction — after that the switches in
    /// the Tools section are there, and a banner that keeps coming back is one
    /// people learn to dismiss without reading.
    ///
    /// Not a sheet. A modal over a menu bar panel is a dialog that blocks the
    /// one surface the user opened to get an answer, and the answer here fits
    /// in two lines.
    @ViewBuilder
    private func onboarding() -> some View {
        let candidates = (client.settings?.tools ?? []).filter { $0.installed && !$0.wired }
        if !onboardingAnswered, !candidates.isEmpty {
            VStack(alignment: .leading, spacing: 6) {
                Text("\(Format.list(candidates.map(\.name))) not routed through IronWire.")
                    .font(.caption)
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: 10) {
                    Button("Point \(candidates.count == 1 ? "it" : "them") here") {
                        onboardingAnswered = true
                        for tool in candidates {
                            setTool(tool, connect: true)
                        }
                    }
                    .disabled(busy)
                    Button("Not now") { onboardingAnswered = true }
                    Spacer()
                }
                .font(.caption2)
                .buttonStyle(.link)
            }
            .padding(8)
            .background(Color.secondary.opacity(0.10), in: RoundedRectangle(cornerRadius: 6))
        }
    }

    // MARK: - Tools

    /// Which agents on this machine actually send their traffic here.
    ///
    /// The one thing the rest of this pane cannot tell you. Every backend can
    /// be healthy, a subscription consented and capacity plentiful, and still
    /// nothing routes — because the agent was never pointed at IronWire and
    /// went straight to the provider instead. A green dot beside a silent
    /// proxy is the most misleading state this app can show, and this section
    /// exists to make it impossible.
    ///
    /// Only tools that are actually here are listed. The daemon reports the
    /// others so a client *can* say "never heard of it", but a dropdown listing
    /// editors you do not have is noise.
    ///
    /// One line, and the list behind it. The whole question is answered by the
    /// label — the names of the agents whose traffic arrives here, or the fact
    /// that there are none — and the switches are for the two occasions anyone
    /// wants them: turning one off, and turning on an agent installed after the
    /// launch that offered to do it. Every tool having its own row and its own
    /// caption was five lines to say what this says in one.
    @ViewBuilder
    private func toolsSection() -> some View {
        let tools = (client.settings?.tools ?? []).filter(\.installed)
        if !tools.isEmpty {
            section("Tools") {
                Menu(routedSummary(tools)) {
                    // A checkmark each, which is what a menu uses to say "on".
                    ForEach(tools) { tool in
                        Toggle(tool.name, isOn: Binding(
                            get: { tool.wired },
                            set: { wanted in setTool(tool, connect: wanted) }
                        ))
                        .help(tool.configPath ?? tool.connectCommand)
                    }
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
                .disabled(busy)
                .accessibilityLabel(Text("Tools routed through IronWire"))
            }
        }
    }

    /// Which agents actually send their traffic here, as the label of the menu
    /// that changes it. Names rather than a count: "Claude Code" is the answer
    /// to "is anything routing", and "2 of 3" is not.
    private func routedSummary(_ tools: [ToolView]) -> String {
        let wired = tools.filter(\.wired).map(\.name)
        return wired.isEmpty ? "none routed here" : wired.joined(separator: ", ")
    }

    /// Wire a tool, or take it back off.
    ///
    /// The one write in this app that changes a file outside `$IRONWIRE_HOME`,
    /// and the checkmark is not always the whole story: a slot the user was
    /// already using is left alone, so the tool can come back wired with part of
    /// its config still pointing somewhere else. That is the one outcome a
    /// checkmark cannot express, so it is the one that gets a sentence.
    private func setTool(_ tool: ToolView, connect: Bool) {
        busy = true
        Task {
            settingsError = nil
            settingsWarning = nil
            switch await client.setTool(id: tool.id, connect: connect) {
            case .success(let outcome):
                settingsWarning = outcome.occupied.isEmpty
                    ? nil
                    : outcome.occupied.joined(separator: "; ")
            case .failure(let failure):
                settingsError = failure.message
            }
            busy = false
        }
    }

    // MARK: - Usage

    /// What IronWire itself has sent this window.
    ///
    /// This is the one block on the screen that shows a percentage without the
    /// provider having reported anything, and it is allowed to because it is
    /// not a claim about the provider (`AGENTS.md` rule 2, the `ironwire_usage`
    /// exception). The guard is that a percentage is never drawn without
    /// `ceiling.describes` next to it saying what it is a percentage *of* —
    /// "your own p90 over 14 sessions" is a different sentence from "42% of
    /// your plan", and only the first one is true.
    @ViewBuilder
    private func usageSection(_ usage: UsageView) -> some View {
        if !usage.sessions.isEmpty {
            section("This \(usage.sessionHours)h window") {
                ForEach(usage.sessions) { session in
                    VStack(alignment: .leading, spacing: 1) {
                        HStack(spacing: 6) {
                            Text(session.backend).font(.caption)
                            Spacer()
                            if let pct = session.usedPct {
                                Text("\(Int(pct.rounded()))%")
                                    .font(.caption)
                                    .foregroundStyle(colour(for: Format.usageLevel(usedPct: pct)))
                            } else {
                                // No ceiling to be a percentage of. The
                                // exchange count is a fact; a percentage here
                                // would be one we made up.
                                Text("\(session.exchanges) exchange(s)")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }
                        if let ceiling = session.ceiling {
                            Text(ceiling.unverified
                                ? "of \(ceiling.describes) (unverified)"
                                : "of \(ceiling.describes)")
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        // The question the block exists to answer, and only
                        // said when the daemon has the rate to answer it.
                        if session.exhaustsBeforeClose, let minutes = session.exhaustsInMinutes {
                            Text("at this rate, reached in \(Format.duration(Int(minutes * 60)))")
                                .font(.caption2)
                                .foregroundStyle(.orange)
                        }
                    }
                }
            }
        }
    }

    // MARK: - Balance

    @ViewBuilder
    private func balanceSection(_ balance: BalanceView) -> some View {
        section("Balance") {
            Text(poolLine(balance))
                .font(.caption)

            ForEach(balance.subscriptionUsed) { use in
                HStack {
                    Text(use.name).font(.caption)
                    Spacer()
                    // `nil` is "the provider has not said", and it says so.
                    // Filling it in would be the same fabrication as a bar
                    // drawn at zero.
                    if let pct = use.usedPct {
                        Text("\(Int(pct.rounded()))% used")
                            .font(.caption)
                            .foregroundStyle(colour(for: Format.usageLevel(usedPct: pct)))
                    } else {
                        Text("not yet reported")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }

            if let cap = balance.spendCap {
                Text("spend today: \(Format.currency(cap.spentUsd)) of \(Format.currency(cap.capUsd)) cap")
                    .font(.caption)
            }
            // Zero is a result, not an absence: it is the sentence "nothing was
            // billed today", which is the point of routing to a subscription.
            if let spend = balance.spendTodayUsd {
                Text("metered spend today: \(Format.currency(spend))")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                Text("metered spend today: not recorded (ledger off)")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func poolLine(_ balance: BalanceView) -> String {
        var parts: [String] = []
        if balance.available > 0 {
            switch balance.freeAvailable {
            case 0:
                parts.append("\(balance.available) pool(s) available")
            case balance.available:
                parts.append("\(balance.available) pool(s) available, all already paid for")
            default:
                parts.append("\(balance.available) pool(s) available (\(balance.freeAvailable) already paid for)")
            }
        }
        // Never folded into "available": the provider has told us nothing, and
        // reporting that as headroom is the fabrication this screen avoids.
        if balance.unknown > 0 { parts.append("\(balance.unknown) not yet reporting") }
        if balance.unavailable > 0 {
            if let at = balance.nextAvailableAt {
                let secs = Int(at.timeIntervalSinceNow)
                parts.append("\(balance.unavailable) unavailable · first back in \(Format.duration(secs))")
            } else {
                parts.append("\(balance.unavailable) unavailable")
            }
        }
        return parts.isEmpty ? "no pools yet" : parts.joined(separator: " · ")
    }

    // MARK: - Route

    @ViewBuilder
    private func routeSection(_ status: StatusView) -> some View {
        if let route = status.lastRoute {
            section("Current route") {
                Text(Format.routeSummary(route))
                    .font(.callout.weight(.medium))
                HStack(spacing: 4) {
                    if let model = route.model {
                        Text(model).font(.caption)
                    }
                    Text("· \(Format.relative(route.at)) ago")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                // The rung comes off the wire. Working "is this degraded" out
                // from backend names here would be a second implementation of a
                // routing question, in a language that cannot see the policy.
                if route.rung.isDegraded {
                    Text(route.rung.label)
                        .font(.caption)
                        .foregroundStyle(route.rung.isUserVisible ? .red : .orange)
                }
            }
        }
    }

    // MARK: - Pin

    private func pinSection(_ status: StatusView) -> some View {
        section("Route to") {
            Menu(status.pin ?? "Automatic") {
                Button("Automatic") { setPin(nil) }
                Divider()
                ForEach(status.backends) { backend in
                    Button(backend.id) { setPin(backend.id) }
                }
            }
            .menuStyle(.borderlessButton)
            .fixedSize()

            if let pinError {
                Text(pinError)
                    .font(.caption)
                    .foregroundStyle(.red)
            }
        }
    }

    private func setPin(_ backend: String?) {
        Task {
            pinError = nil
            // The daemon validates and stores; the menu shows whatever the next
            // poll reports, not what we asked for.
            if case .failure(let error) = await client.pin(backend: backend) {
                pinError = error.message
            }
        }
    }

    // MARK: - Privacy

    /// The filter, as a control when the daemon has told us what is selectable
    /// and as its own words when it has not.
    ///
    /// Either way this says what the filter is *doing*, never what the user is
    /// safe from: no shield, no lock, no "protected" (`docs/TRUST.md` I7).
    @ViewBuilder
    private func privacySection(_ status: StatusView) -> some View {
        if let privacy = client.settings?.privacy, !privacy.options.isEmpty {
            section("Privacy filter") {
                // Nothing but the control. What each mode substitutes, and why a
                // dimmed one cannot be picked, are both carried as the segment's
                // tooltip and its VoiceOver hint — reachable, and not four lines
                // of prose in a dropdown read at a glance.
                //
                // This is weaker than drawing the reason: a user who never
                // hovers sees a mode they cannot select and no explanation. The
                // stronger version is one `ForEach` in `privacySection`. It is
                // also mostly moot once a trusted backend ships by default,
                // because then `full` is selectable and there is no reason to
                // show.
                privacyControl(privacy)
            }
        } else if let privacy = status.privacy {
            section("Privacy filter") {
                Text(privacy)
                    .font(.caption)
                    .textSelection(.enabled)
            }
        }
    }

    /// The ladder as one control.
    ///
    /// Hand-rolled rather than a segmented `Picker` because `Picker` cannot
    /// disable an individual segment, and `full` being unselectable — with its
    /// reason — is the one thing this control has to get right.
    private func privacyControl(_ privacy: PrivacySettingsView) -> some View {
        HStack(spacing: 2) {
            ForEach(privacy.options) { option in
                let isCurrent = option.id == privacy.mode
                Button {
                    apply(mode: option.id)
                } label: {
                    Text(option.id)
                        .font(.caption2.weight(isCurrent ? .semibold : .regular))
                        .lineLimit(1)
                        .minimumScaleFactor(0.8)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 4)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .background(
                    RoundedRectangle(cornerRadius: 5)
                        .fill(isCurrent ? Color.accentColor.opacity(0.30) : Color.clear)
                )
                .disabled(!option.selectable || busy || isCurrent)
                .opacity(option.selectable ? 1 : 0.45)
                .help(option.describes)
                // Without this the segments are four unlabelled buttons to
                // VoiceOver, which is how the mode choice becomes unusable.
                .accessibilityLabel(Text(option.id))
                .accessibilityHint(Text(option.selectable ? option.describes : (option.unavailableBecause ?? option.describes)))
                .accessibilityAddTraits(isCurrent ? [.isSelected] : [])
            }
        }
        .padding(2)
        .background(RoundedRectangle(cornerRadius: 7).fill(Color.secondary.opacity(0.12)))
    }

    private func apply(mode: String) {
        busy = true
        Task {
            settingsError = nil
            settingsWarning = nil
            switch await client.setPrivacyMode(mode) {
            case .success(let outcome):
                // Applied, but not saved. Said plainly rather than swallowed:
                // the user would otherwise find the old mode back after a
                // restart with no idea why.
                settingsWarning = outcome.warning
            case .failure(let failure):
                settingsError = failure.message
            }
            busy = false
        }
    }

    // MARK: - Update

    @ViewBuilder
    private func updateSection(_ update: UpdateStatus) -> some View {
        switch update {
        case .available(let latest, let summary, let command):
            section("Update") {
                // A link, and nothing that could apply it. See the file header.
                Link("ironwire \(latest) is available", destination: releasesURL)
                    .font(.caption)
                if let summary {
                    Text(summary).font(.caption).foregroundStyle(.secondary)
                }
                if let command {
                    Text(command)
                        .font(.caption.monospaced())
                        .textSelection(.enabled)
                        .foregroundStyle(.secondary)
                }
            }
        case .unsupported(let latest, let minimum, let command):
            section("Update") {
                Text("This build is below the supported floor (\(minimum)); providers may have changed in ways it does not handle.")
                    .font(.caption)
                    .foregroundStyle(.red)
                Link("ironwire \(latest) is available", destination: releasesURL)
                    .font(.caption)
                if let command {
                    Text(command)
                        .font(.caption.monospaced())
                        .textSelection(.enabled)
                        .foregroundStyle(.secondary)
                }
            }
        // `unrecognised` is a state from a newer daemon. Showing nothing is the
        // only honest option — the alternative is guessing what it meant.
        case .upToDate, .unknown, .unrecognised:
            EmptyView()
        }
    }

    private var releasesURL: URL {
        URL(string: "https://github.com/nearai/ironwire/releases") ?? URL(fileURLWithPath: "/")
    }

    // MARK: - Footer

    /// Settings and the way out, as menu items.
    ///
    /// Every menu bar app on the machine ends this way — a couple of checkmarked
    /// preferences and Quit, all the same shape, none of them blue. A row of
    /// link-styled buttons was neither a menu nor a toolbar.
    private var footer: some View {
        VStack(alignment: .leading, spacing: 0) {
            MenuRow { notifier.enabled.toggle() } label: {
                checked("Notify on family changes and failures", on: notifier.enabled)
            }
            MenuRow { loginItem.set(!loginItem.isOn) } label: {
                checked("Open at login", on: loginItem.isOn)
            }
            // The one caption down here, and only for the states that are
            // neither on nor off: a login item awaiting approval in System
            // Settings, or one that could not be registered at all.
            if let detail = loginItem.detail {
                Text(detail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, 26)
                    .padding(.bottom, 4)
            }
            // The way out of this pane and into `curl`, for the questions a
            // dropdown is the wrong shape for. Offered only while something is
            // answering, and the port is the daemon's own rather than the one
            // this app guessed at launch.
            if let port = client.status?.port {
                MenuRow { copyControlURL(port: port) } label: {
                    labelled(urlCopied ? "Control URL copied" : "Copy control URL")
                }
                // The token is deliberately not on the clipboard with it. It is
                // the credential for changing where someone's traffic goes, and
                // a second copy of it living in the pasteboard is worse than a
                // sentence naming the file it is in.
                .help("\(Discovery.controlURL(port: port).absoluteString) — calls need the bearer token from \(Discovery.home().path)/control.token")
            }
            // Only offered when there is something to open. A menu item that
            // opens nothing is worse than one that is absent — a foreground
            // `ironwire serve` logs to its own terminal and has no file.
            if let log = Discovery.brewLog() {
                MenuRow { NSWorkspace.shared.open(log) } label: {
                    labelled("Open log")
                }
            }
            MenuRow { NSApp.terminate(nil) } label: {
                labelled("Quit IronWire")
            }
            .keyboardShortcut("q")
        }
        .padding(6)
    }

    private func copyControlURL(port: Int) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(
            Discovery.controlURL(port: port).absoluteString, forType: .string)
        urlCopied = true
        Task {
            try? await Task.sleep(for: .seconds(2))
            urlCopied = false
        }
    }

    /// A menu item's label, indented past the checkmark gutter so the whole
    /// footer reads down one edge.
    private func labelled(_ title: String) -> some View {
        checked(title, on: false)
    }

    private func checked(_ title: String, on: Bool) -> some View {
        HStack(spacing: 6) {
            Image(systemName: "checkmark")
                .font(.caption.weight(.semibold))
                .opacity(on ? 1 : 0)
                .frame(width: 12)
            Text(title)
        }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(on ? [.isSelected] : [])
    }

    // MARK: - Chrome

    private func section<Content: View>(
        _ title: String, @ViewBuilder content: () -> Content
    ) -> some View {
        LazyVStack(alignment: .leading, spacing: 6) {
            Text(title.uppercased())
                .font(.caption2.weight(.semibold))
                .foregroundStyle(.secondary)
            content()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// A row that behaves like a menu item.
///
/// `MenuBarExtra(.window)` draws a panel, not a menu — the style is what lets a
/// capacity bar be absent, see `IronWireApp.swift` — so there are no real menu
/// items in here and the affordance has to be built: full width, left aligned,
/// highlighted under the pointer, and the same shape whether it toggles
/// something or quits.
private struct MenuRow<Label: View>: View {
    private let action: () -> Void
    private let label: Label
    @State private var hovering = false

    init(action: @escaping () -> Void, @ViewBuilder label: () -> Label) {
        self.action = action
        self.label = label()
    }

    var body: some View {
        Button(action: action) {
            label
                .font(.callout)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 6)
                .padding(.vertical, 4)
                // Without this the row only responds where the text is, which
                // is not how any other menu on the machine behaves.
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .background(
            RoundedRectangle(cornerRadius: 5)
                .fill(hovering ? Color.accentColor.opacity(0.25) : .clear)
        )
        .onHover { hovering = $0 }
    }
}
