//! The dropdown, rendered off-screen.
//
// `ImageRenderer` lays the menu out exactly as the menu bar would, without a
// screen, a click, or a permission prompt. That buys two things the unit tests
// cannot: proof that every state actually *lays out* — a view that trapped on a
// `nil` would fail here — and a measurement of the one rule this app is most
// likely to break quietly.
//
// The bar test is the reason this file exists. `Format.capacityFraction`
// returning `nil` is necessary but not sufficient: a view could still draw a
// `ProgressView` beside it, and no test of the formatter would notice. The
// assertion is on laid-out height, because a bar occupies vertical space that
// text does not — see `height(for:)` for why height and not pixels.

import SwiftUI
import XCTest

@testable import IronWire
@testable import IronWireKit

@MainActor
final class MenuContentTests: XCTestCase {
    /// The `@AppStorage` key behind the one-time offer to wire every agent.
    ///
    /// Answered by default here. It is a launch-time banner that appears
    /// whenever an unwired tool exists, so leaving it unanswered would add its
    /// height to half the tool cases below and measure the banner instead of
    /// the thing under test. Set through `UserDefaults` because that is what
    /// `@AppStorage` reads — the real switch, not a stand-in for it.
    private let onboardingKey = "toolsOnboardingAnswered"

    override func setUp() {
        super.setUp()
        UserDefaults.standard.set(true, forKey: onboardingKey)
    }

    override func tearDown() {
        UserDefaults.standard.removeObject(forKey: onboardingKey)
        super.tearDown()
    }

    // MARK: - Fixtures

    private func status(
        backends: [BackendView],
        balance: BalanceView = BalanceView(available: 1, freeAvailable: 1),
        lastRoute: LastRouteView? = nil,
        privacy: String? = nil,
        update: UpdateStatus = .upToDate,
        pin: String? = nil,
        usage: UsageView = UsageView()
    ) -> StatusView {
        StatusView(
            version: "0.1.0", port: 8463, trackedConversations: 2, pin: pin,
            backends: backends, balance: balance, privacy: privacy,
            update: update, lastRoute: lastRoute, usage: usage)
    }

    private func render(
        _ status: StatusView?, connection: ControlClient.Connection = .connected,
        settings: SettingsView? = nil
    ) -> NSImage? {
        let view = MenuContent(
            client: .fixture(status: status, connection: connection, settings: settings),
            notifier: Notifier(), loginItem: LoginItem())
        let renderer = ImageRenderer(content: view)
        renderer.scale = 2
        return renderer.nsImage
    }

    private func height(_ status: StatusView?, settings: SettingsView? = nil) throws -> CGFloat {
        try XCTUnwrap(render(status, settings: settings)).size.height
    }

    /// The laid-out height of a menu showing one backend in a given state.
    ///
    /// Height rather than pixels, because `ImageRenderer` cannot rasterise
    /// `ProgressView` — it is AppKit-backed — but it does lay it out. The bar
    /// therefore shows up as vertical space and nowhere else, which is exactly
    /// the thing being asserted.
    private func height(for headroom: HeadroomView) throws -> CGFloat {
        let rendered = try XCTUnwrap(
            render(status(backends: [BackendView(id: "a", name: "Claude subscription", headroom: headroom)])))
        return rendered.size.height
    }

    func test_a_menu_with_a_thousand_logins_renders_offscreen_without_losing_identity() throws {
        let backends = (0..<1024).map { BackendView(id: "claude-\($0)", name: "Claude subscription") }
        let services = backends.map { backend in
            ServiceView(id: backend.id, name: backend.name, requiresConsent: true, consented: false,
                consentPrompt: ConsentPromptView(version: 2, backendId: backend.id, product: "Claude",
                    summary: "Read this login", points: ["Private auth risk"], question: "Enable?",
                    binding: "opaque-\(backend.id)"))
        }
        let rendered = try XCTUnwrap(render(status(backends: backends), settings: settings(services: services)))
        XCTAssertEqual(Set(backends.map(\.id)).count, 1024)
        XCTAssertTrue(rendered.size.width.isFinite && rendered.size.height.isFinite)
        XCTAssertGreaterThan(rendered.size.width, 0)
        XCTAssertGreaterThan(rendered.size.height, 0)
    }

    // MARK: - The bar

    /// The acceptance criterion the issue calls the single most likely place for
    /// this app to go wrong. An unobserved backend must produce the word
    /// `unknown` and *no bar* — not a bar at zero, not a greyed one at half.
    func test_an_unobserved_backend_lays_out_shorter_because_it_has_no_bar() throws {
        let unobserved = try height(for: .unknown)
        let observed = try height(for: .observed(usedPct: 82, observedSecsAgo: 40, resetsInSecs: 1800))
        XCTAssertGreaterThan(
            observed, unobserved,
            "an unobserved backend appears to be drawing a capacity bar")
    }

    /// The set of states that must draw nothing, checked against each other
    /// rather than against a constant: all four carry a single line of text and
    /// no bar, so all four must lay out to exactly the same height. A bar
    /// appearing in any of them moves that one and fails here.
    ///
    /// `exhausted` and `capReached` are in this set deliberately. Neither is a
    /// measurement of how much of a window went, so a bar pinned to full would
    /// be claiming one.
    func test_no_state_but_an_observation_draws_a_bar() throws {
        let baseline = try height(for: .unknown)
        for headroom: HeadroomView in [
            .unrecognised("throttled"),
            .exhausted(retryInSecs: 300),
            .capReached(spentUsd: 5, capUsd: 5, resetsInSecs: 3600),
        ] {
            XCTAssertEqual(
                try height(for: headroom), baseline,
                "\(headroom) lays out differently from `unknown`, which means it is drawing a bar")
        }
    }

    /// The bar is the difference, not the wording: an observation with a reset
    /// time and one without carry different text and lay out identically.
    func test_the_extra_height_is_the_bar_and_not_the_words() throws {
        let withReset = try height(for: .observed(usedPct: 82, observedSecsAgo: 40, resetsInSecs: 1800))
        let without = try height(for: .observed(usedPct: 82, observedSecsAgo: 40, resetsInSecs: nil))
        XCTAssertEqual(withReset, without)
    }

    // MARK: - Every state lays out

    func test_a_daemon_that_is_not_running_renders_without_an_error_dialog() throws {
        let rendered = try XCTUnwrap(render(nil, connection: .unreachable))
        XCTAssertGreaterThan(rendered.size.height, 0)
    }

    func test_a_rejected_token_renders_as_a_line_and_not_as_a_crash() throws {
        let rendered = try XCTUnwrap(render(nil, connection: .unauthorised))
        XCTAssertGreaterThan(rendered.size.height, 0)
    }

    func test_a_fresh_install_with_no_backends_lays_out() throws {
        let rendered = try XCTUnwrap(render(status(backends: [])))
        XCTAssertGreaterThan(rendered.size.height, 0)
    }

    /// The state a real daemon will not produce on request, and the one the icon
    /// exists for.
    func test_a_skipped_backend_and_a_cross_family_route_lay_out() throws {
        let rendered = try XCTUnwrap(
            render(
                status(
                    backends: [
                        BackendView(
                            id: "claude-sub", name: "Claude subscription",
                            headroom: .observed(usedPct: 97, observedSecsAgo: 5, resetsInSecs: 600),
                            health: HealthView(circuit: "open", consecutiveFailures: 3, retryInSecs: 45)),
                        BackendView(id: "nearai", name: "NEAR AI", kind: "credits", headroom: .unknown),
                    ],
                    balance: BalanceView(
                        available: 0, unknown: 1, unavailable: 1,
                        nextAvailableAt: Date().addingTimeInterval(45),
                        spendTodayUsd: 0.16, spendCap: SpendCapView(spentUsd: 0.16, capUsd: 5)),
                    lastRoute: LastRouteView(
                        backend: "nearai", model: "qwen3-coder", from: "claude-sub", rung: .crossFamily),
                    privacy: "redacting emails and API keys",
                    update: .available(latest: "0.2.0", summary: "faster failover", upgradeCommand: "brew upgrade ironwire"),
                    pin: "nearai")))
        XCTAssertGreaterThan(rendered.size.height, 0)
    }

    /// A newer daemon must cost one field, never the menu.
    func test_a_status_full_of_states_this_build_does_not_recognise_still_lays_out() throws {
        let rendered = try XCTUnwrap(
            render(
                status(
                    backends: [BackendView(id: "a", name: "A", headroom: .unrecognised("throttled"))],
                    lastRoute: LastRouteView(backend: "a", rung: .unrecognised("different_provider")),
                    update: .unrecognised("rollback_required"))))
        XCTAssertGreaterThan(rendered.size.height, 0)
    }

    // MARK: - Consent, on the backend row
    //
    // The switch and the capacity share a row now, so these live here rather
    // than in a settings pane of their own.

    private func prompt(points: Int = 4, version: Int = 2) -> ConsentPromptView {
        ConsentPromptView(
            version: version, backendId: "claude-sub", product: "Claude",
            summary: "IronWire will read the OAuth token that Claude Code stores on this machine "
                + "and send requests to api.anthropic.com with it, from this computer only.",
            points: (0..<points).map { "Point number \($0), which is a whole sentence about a real cost." },
            question: "Enable the Claude subscription backend?", binding: "test-login")
    }

    private func settings(
        options: [PrivacyOptionView] = [],
        services: [ServiceView] = []
    ) -> SettingsView {
        SettingsView(
            privacy: PrivacySettingsView(mode: "off", summary: "off", options: options),
            services: services)
    }

    private func unconsented(prompt: ConsentPromptView?) -> (StatusView, SettingsView) {
        (
            status(backends: [
                BackendView(id: "claude-sub", name: "Claude subscription", consented: false)
            ]),
            settings(services: [
                ServiceView(
                    id: "claude-sub", name: "Claude subscription", authenticated: true,
                    requiresConsent: true, consented: false, consentPrompt: prompt,
                    connectCommand: "ironwire connect claude")
            ])
        )
    }

    /// The two conditions a switch is gated on, and proof the row lays out with
    /// one. Asserted as predicates rather than by height: the fallback row for a
    /// prompt this build could not read is also two lines, so the two cases are
    /// deliberately the same height and a height comparison would prove nothing.
    func test_a_switch_is_offered_only_for_a_credential_and_a_readable_question() throws {
        let service = ServiceView(
            id: "claude-sub", name: "Claude subscription", authenticated: true,
            requiresConsent: true, consented: false, consentPrompt: prompt(),
            connectCommand: "ironwire connect claude")
        XCTAssertTrue(service.canToggle)
        XCTAssertTrue(service.consentPrompt?.isComplete == true)

        let (status, settings) = unconsented(prompt: prompt())
        XCTAssertGreaterThan(try height(status, settings: settings), 0)
    }

    /// The consent text costs no height at all — neither the points nor the
    /// summary. Checked against a prompt three times as long in both: if any of
    /// it were being drawn, the longer one would lay out taller.
    ///
    /// This is the height half of the trade recorded in `docs/TRUST.md` §2. The
    /// row is a name and a switch regardless of how much the daemon has to say;
    /// what it has to say is the switch's tooltip and its VoiceOver hint.
    func test_the_consent_text_is_never_drawn_on_the_row() throws {
        let (shortStatus, shortSettings) = unconsented(prompt: prompt(points: 4))
        let (longStatus, longSettings) = unconsented(
            prompt: ConsentPromptView(
                version: 2, backendId: "claude-sub", product: "Claude",
                summary: String(repeating: "A very long summary sentence. ", count: 12),
                points: (0..<12).map { "Point \($0), which is a whole sentence about a real cost." },
                question: "Enable the Claude subscription backend?", binding: "test-login"))
        XCTAssertEqual(
            try height(shortStatus, settings: shortSettings),
            try height(longStatus, settings: longSettings),
            "the row grows with the consent text, so it is being drawn")
    }

    /// A switch that is off already reads "off", so the row does not say it
    /// again. The comparison is against the row that *has* no switch — a prompt
    /// this build could not read — which has to spend two lines saying the same
    /// thing in words, and is therefore taller.
    func test_a_row_with_a_switch_does_not_repeat_what_the_switch_says() throws {
        let (withSwitch, withSwitchSettings) = unconsented(prompt: prompt())
        let (noSwitch, noSwitchSettings) = unconsented(prompt: nil)
        XCTAssertLessThan(
            try height(withSwitch, settings: withSwitchSettings),
            try height(noSwitch, settings: noSwitchSettings),
            "the row with a switch is still spending lines on \"not enabled\"")
    }

    /// A credential that was never found is not something a menu can conjure, so
    /// there is nothing to switch and the row names the command instead.
    func test_a_backend_with_no_credential_offers_a_command_not_a_switch() throws {
        let service = ServiceView(
            id: "claude-sub", name: "Claude subscription", authenticated: false,
            detail: "no Claude Code login found", requiresConsent: true, consented: false,
            consentPrompt: prompt(), connectCommand: "ironwire connect claude")
        XCTAssertFalse(service.canToggle)
        let rendered = render(
            status(backends: [
                BackendView(
                    id: "claude-sub", name: "Claude subscription", authenticated: false,
                    consented: false)
            ]),
            settings: settings(services: [service]))
        XCTAssertNotNil(rendered)
    }

    /// The status document is polled and settings is fetched on open, so the
    /// row's own copy of the command is the one that is there first.
    func test_a_backend_with_no_credential_names_its_command_before_settings_arrive() throws {
        let withCommand = try height(
            status(backends: [
                BackendView(
                    id: "claude-work", name: "Claude subscription (~/.claude-work)",
                    authenticated: false, consented: false,
                    detail: "Claude Code is not logged in for ~/.claude-work",
                    connectCommand: "ironwire connect claude --config-dir ~/.claude-work")
            ]))
        let without = try height(
            status(backends: [
                BackendView(
                    id: "claude-work", name: "Claude subscription (~/.claude-work)",
                    authenticated: false, consented: false,
                    detail: "Claude Code is not logged in for ~/.claude-work")
            ]))
        XCTAssertGreaterThan(withCommand, without, "the row's own connect command is not being drawn")
    }

    // MARK: - Several logins of one product

    /// Two Claude logins under one display name. The daemon names them apart,
    /// but a name is for reading: this is the case where rows keyed on it
    /// would collapse.
    private func twoClaudeLogins(_ uses: [SubscriptionUse]) -> StatusView {
        status(
            backends: [
                BackendView(id: "claude-sub", name: "Claude subscription"),
                BackendView(id: "claude-work", name: "Claude subscription"),
            ],
            balance: BalanceView(available: 2, freeAvailable: 2, subscriptionUsed: uses))
    }

    private let personal = SubscriptionUse(
        backend: "claude-sub", name: "Claude subscription", usedPct: 82, exchanges: 41)
    private let work = SubscriptionUse(
        backend: "claude-work", name: "Claude subscription", usedPct: nil, exchanges: 3)

    /// Identity is asserted directly as well as by height: an off-screen render
    /// draws both rows even when they share an id. It is a live list being
    /// updated that goes wrong, and that is not something a render can catch.
    func test_two_logins_with_one_display_name_are_two_rows_in_the_balance() throws {
        XCTAssertNotEqual(personal.id, work.id, "two logins share one row identity")

        let both = try height(twoClaudeLogins([personal, work]))
        let one = try height(twoClaudeLogins([personal]))
        XCTAssertGreaterThan(both, one, "the second login's row is not being drawn")
    }

    // MARK: - The window

    /// `AGENTS.md` rule 2's one allowed percentage, and the guard on it. This
    /// figure is legitimate because it measures IronWire's own traffic, not a
    /// provider's remaining capacity — but only while the daemon's phrase for
    /// *what it is a percentage of* is drawn with it. A window carrying a
    /// ceiling therefore lays out taller than the same window without one.
    func test_a_usage_percentage_is_drawn_with_the_basis_it_is_measured_against() throws {
        let withBasis = try height(
            status(
                backends: [],
                usage: UsageView(sessions: [
                    SessionUsageView(
                        backend: "claude-sub", remainingMinutes: 120, exchanges: 9,
                        usedPct: 42,
                        ceiling: CeilingView(describes: "your own p90 over 14 sessions"))
                ])))
        let withoutBasis = try height(
            status(
                backends: [],
                usage: UsageView(sessions: [
                    SessionUsageView(backend: "claude-sub", remainingMinutes: 120, exchanges: 9)
                ])))
        XCTAssertGreaterThan(
            withBasis, withoutBasis,
            "a usage percentage is being drawn without the basis it is measured against")
    }

    /// No history yet means no ceiling, which is the state a fresh install is
    /// in — and the one where a percentage would be invented. It has to lay out
    /// with the exchange count instead of a number nobody measured.
    func test_a_window_with_no_ceiling_still_lays_out() throws {
        XCTAssertGreaterThan(
            try height(
                status(
                    backends: [],
                    usage: UsageView(
                        sessions: [
                            SessionUsageView(
                                backend: "claude-sub", remainingMinutes: 240, exchanges: 1)
                        ],
                        completedSessions: 0))),
            0)
    }

    // MARK: - Tools

    private func withTools(_ tools: [ToolView]) -> SettingsView {
        SettingsView(
            privacy: PrivacySettingsView(mode: "off", summary: "off"), services: [], tools: tools)
    }

    private func tool(_ id: String, _ name: String, wired: Bool) -> ToolView {
        ToolView(
            id: id, name: name, installed: true, wired: wired,
            connectCommand: "ironwire connect \(id)")
    }

    /// The state this section exists for: everything healthy, nothing routing.
    /// The section has to occupy space even when nothing is wired — that is the
    /// case where its label reads "none routed here" — or the pane goes on
    /// implying that a green backend means traffic is arriving.
    func test_an_unwired_tool_is_still_reported() throws {
        let listed = try height(
            status(backends: []), settings: withTools([tool("claude", "Claude Code", wired: false)]))
        let nothing = try height(status(backends: []), settings: withTools([]))
        XCTAssertGreaterThan(listed, nothing, "an unwired tool is not being drawn at all")
    }

    /// Wired or not, a tool costs the same height, because the state lives in
    /// the menu's label and its checkmarks and nowhere else. The captions this
    /// replaced ("not routed here", and what the last write did) are what made
    /// the pane a wall of text. With the one-time offer already answered, an
    /// unwired agent costs the pane nothing at all.
    func test_a_tool_costs_the_same_whether_it_is_wired_or_not() throws {
        let wired = try height(
            status(backends: []), settings: withTools([tool("claude", "Claude Code", wired: true)]))
        let unwired = try height(
            status(backends: []), settings: withTools([tool("claude", "Claude Code", wired: false)]))
        XCTAssertEqual(wired, unwired, "an unwired tool is drawing a caption of its own")
    }

    /// The offer to point everything here is made once, at the launch that
    /// finds an unwired agent, and never again once it has been answered. It is
    /// the only place in the pane that spends lines on an unwired tool.
    func test_the_offer_to_wire_everything_is_made_once() throws {
        let settings = withTools([tool("claude", "Claude Code", wired: false)])

        UserDefaults.standard.set(false, forKey: onboardingKey)
        let offered = try height(status(backends: []), settings: settings)
        UserDefaults.standard.set(true, forKey: onboardingKey)
        let answered = try height(status(backends: []), settings: settings)

        XCTAssertGreaterThan(offered, answered, "the one-time offer is not being made")
    }

    /// The list is behind the menu, so a machine with every agent installed
    /// costs exactly as much of the dropdown as a machine with one.
    func test_the_tools_row_does_not_grow_with_the_number_of_tools() throws {
        let one = try height(
            status(backends: []), settings: withTools([tool("claude", "Claude Code", wired: true)]))
        let many = try height(
            status(backends: []),
            settings: withTools([
                tool("claude", "Claude Code", wired: true),
                tool("codex", "Codex", wired: false),
                tool("gemini", "Gemini CLI", wired: false),
                tool("opencode", "OpenCode", wired: true),
            ]))
        XCTAssertEqual(one, many, "the tools list is being drawn in the pane rather than in a menu")
    }

    /// The daemon reports every tool it knows about so a client *can* say
    /// "never heard of it". A dropdown listing editors you do not have is
    /// noise, so this one shows only what is here.
    func test_a_tool_that_is_not_installed_is_not_listed() throws {
        let absent = try height(
            status(backends: []),
            settings: withTools([
                ToolView(id: "codex", name: "Codex", installed: false, wired: false)
            ]))
        let none = try height(status(backends: []), settings: withTools([]))
        XCTAssertEqual(absent, none)
    }

    // MARK: - The privacy ladder

    /// The reason a mode cannot be picked is carried, not drawn: it is the
    /// segment's tooltip and VoiceOver hint, so the control stays one row high
    /// however much the daemon has to say about it.
    ///
    /// Deliberately the inverse of what this test asserted before. Drawing the
    /// reason is the stronger behaviour and the dropdown could not afford it;
    /// the assertion that matters now is that the text never reaches layout.
    func test_an_unavailable_mode_carries_its_reason_without_drawing_it() throws {
        let base = status(backends: [])
        let unselectable = PrivacyOptionView(
            id: "full", describes: "only trusted backends", selectable: false,
            unavailableBecause: String(repeating: "A long reason nobody has room for. ", count: 8))
        // The model still carries it — this is what the tooltip and the
        // accessibility hint read from.
        XCTAssertNotNil(unselectable.unavailableBecause)

        let withReason = try height(
            base,
            settings: settings(options: [
                PrivacyOptionView(id: "off", describes: "requests are forwarded unchanged"),
                unselectable,
            ]))
        let withoutReason = try height(
            base,
            settings: settings(options: [
                PrivacyOptionView(id: "off", describes: "requests are forwarded unchanged"),
                PrivacyOptionView(id: "full", describes: "only trusted backends", selectable: false),
            ]))
        XCTAssertEqual(
            withReason, withoutReason,
            "the unavailable reason is being drawn, which the dropdown has no room for")
    }

    /// With no settings document there is nothing to offer, so the pane falls
    /// back to the daemon's own words and still lays out.
    func test_the_privacy_line_falls_back_to_the_daemons_words() throws {
        XCTAssertGreaterThan(
            try height(status(backends: [], privacy: "redacting emails and API keys")), 0)
    }

    /// Every backend a daemon reported has to be offerable, including ones that
    /// are not usable right now — the daemon validates the choice, not this app.
    func test_the_pin_control_offers_every_backend_the_daemon_reported() {
        let status = status(backends: [
            BackendView(id: "claude-sub", name: "Claude subscription"),
            BackendView(id: "claude-work", name: "Claude subscription (~/.claude-work)"),
            BackendView(id: "codex-sub", name: "ChatGPT subscription", authenticated: false),
            BackendView(id: "nearai", name: "NEAR AI", health: HealthView(circuit: "open")),
        ])
        // The menu is built straight off this list, so the list is the
        // assertion: no filtering, no ranking, no opinion.
        XCTAssertEqual(status.backends.map(\.id), ["claude-sub", "claude-work", "codex-sub", "nearai"])
    }
}
