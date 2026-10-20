//! Reading the settings surface.
//
// The rules under test are all of the form "the daemon decided this, and the app
// must not decide it again". A settings screen is where that discipline is
// hardest to keep, because every one of these looks like something a client
// could work out for itself.

import XCTest

@testable import IronWireKit

final class SettingsTests: XCTestCase {
    private func decode(_ json: String) throws -> SettingsView {
        try controlDecoder().decode(SettingsView.self, from: Data(json.utf8))
    }

    func test_a_settings_document_decodes_every_field_the_pane_shows() throws {
        let settings = try decode(Fixtures.settings)

        XCTAssertEqual(settings.privacy.mode, "pii")
        XCTAssertEqual(settings.privacy.summary, "pii + 1 named value(s)")
        XCTAssertEqual(settings.privacy.options.count, 4)
        XCTAssertEqual(settings.services.count, 2)
        XCTAssertEqual(settings.services[0].id, "claude-sub")
        XCTAssertTrue(settings.services[0].requiresConsent)
        XCTAssertFalse(settings.services[0].consented)
    }

    /// The rule the app must never re-derive. Whether `full` is offerable is
    /// `full_is_blocked`'s business, and the daemon sends the answer with the
    /// sentence a greyed-out segment is greyed out with.
    func test_an_unselectable_mode_arrives_with_the_reason_it_is_unselectable() throws {
        let settings = try decode(Fixtures.settings)
        let full = try XCTUnwrap(settings.privacy.options.first { $0.id == "full" })
        XCTAssertFalse(full.selectable)
        XCTAssertEqual(
            full.unavailableBecause,
            "`full` routes only to backends you have named as acceptable, and none are named.")
    }

    /// An older daemon that does not send the field has no restriction to
    /// report, so the option stands rather than being greyed out for a reason
    /// nobody can name.
    func test_a_mode_with_no_selectable_field_is_treated_as_selectable() throws {
        let settings = try decode(
            #"{"privacy":{"mode":"off","summary":"off","options":[{"id":"pii","describes":"x"}]}}"#)
        let option = try XCTUnwrap(settings.privacy.options.first)
        XCTAssertTrue(option.selectable)
        XCTAssertNil(option.unavailableBecause)
    }

    /// A mode this build has never heard of still renders as itself, rather than
    /// disappearing from a list the daemon says is complete.
    func test_a_mode_from_a_newer_daemon_still_appears() throws {
        let settings = try decode(
            #"{"privacy":{"mode":"paranoid","summary":"paranoid","options":[{"id":"paranoid","describes":"everything","selectable":true}]}}"#)
        XCTAssertEqual(settings.privacy.mode, "paranoid")
        XCTAssertEqual(settings.privacy.options.first?.id, "paranoid")
    }

    func test_a_settings_document_missing_everything_optional_still_decodes() throws {
        let settings = try decode("{}")
        XCTAssertEqual(settings.privacy.mode, "off")
        XCTAssertTrue(settings.privacy.options.isEmpty)
        XCTAssertTrue(settings.services.isEmpty)
    }

    // MARK: - Consent

    func test_the_consent_prompt_arrives_whole() throws {
        let settings = try decode(Fixtures.settings)
        let prompt = try XCTUnwrap(settings.services[0].consentPrompt)
        XCTAssertEqual(prompt.version, 1)
        XCTAssertEqual(prompt.backendId, "claude-sub")
        XCTAssertTrue(prompt.summary.contains("api.anthropic.com"))
        XCTAssertEqual(prompt.points.count, 4)
        XCTAssertTrue(prompt.question.contains("Claude"))
        XCTAssertTrue(prompt.isComplete)
    }

    /// A prompt that arrived without its points is a button with a title, not a
    /// consent screen. Better to send the user to the CLI than to collect an
    /// answer to a question we failed to ask.
    func test_a_prompt_missing_its_substance_is_not_presentable() {
        let bare = ConsentPromptView(
            version: 1, backendId: "claude-sub", product: "Claude",
            summary: "", points: [], question: "Enable it?")
        XCTAssertFalse(bare.isComplete)

        let unversioned = ConsentPromptView(
            version: 0, backendId: "claude-sub", product: "Claude",
            summary: "a summary", points: ["a point"], question: "Enable it?")
        XCTAssertFalse(unversioned.isComplete, "an answer has to belong to a version")
    }

    func test_the_prompt_login_binding_is_decoded_verbatim() throws {
        let binding = "opaque-login-identity"
        let data = Data("""
            {"version":2,"backend_id":"claude-sub","binding":"\(binding)",
             "product":"Claude","summary":"summary","points":["risk"],"question":"Enable?"}
            """.utf8)
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let prompt = try decoder.decode(ConsentPromptView.self, from: data)
        XCTAssertEqual(prompt.binding, binding)
    }

    // MARK: - What the app may act on

    /// Consent is the only thing here the daemon can act on directly. A missing
    /// credential comes from the user logging into Claude Code or exporting a
    /// key — a menu cannot conjure one, and offering to would be a lie.
    func test_only_a_consent_gate_with_a_credential_behind_it_is_toggleable() {
        let ready = ServiceView(
            id: "claude-sub", name: "Claude", authenticated: true,
            requiresConsent: true, consented: false)
        XCTAssertTrue(ready.canToggle)

        let noCredential = ServiceView(
            id: "claude-sub", name: "Claude", authenticated: false,
            requiresConsent: true, consented: false)
        XCTAssertFalse(noCredential.canToggle, "a menu cannot log anybody in")

        let notGated = ServiceView(
            id: "nearai", name: "NEAR AI", kind: "credits", authenticated: true,
            requiresConsent: false, consented: true)
        XCTAssertFalse(notGated.canToggle, "there is no consent gate to toggle")
    }

    func test_a_service_that_cannot_be_toggled_names_the_command_instead() throws {
        let settings = try decode(Fixtures.settings)
        let nearai = try XCTUnwrap(settings.services.first { $0.id == "nearai" })
        XCTAssertFalse(nearai.canToggle)
        XCTAssertEqual(nearai.connectCommand, "ironwire connect near")
    }
}

extension SettingsTests {
    enum Fixtures {
        /// Shaped as `control.rs` serialises it.
        static let settings = #"""
        {
          "privacy": {
            "mode": "pii",
            "summary": "pii + 1 named value(s)",
            "options": [
              {"id": "off", "describes": "off — requests are forwarded unchanged", "selectable": true, "unavailable_because": null},
              {"id": "credentials", "describes": "credentials: API keys, tokens, private keys, and named values", "selectable": true, "unavailable_because": null},
              {"id": "pii", "describes": "credentials, plus deterministic PII", "selectable": true, "unavailable_because": null},
              {"id": "full", "describes": "credentials and deterministic PII, and only trusted backends", "selectable": false,
               "unavailable_because": "`full` routes only to backends you have named as acceptable, and none are named."}
            ],
            "trusted_backends": ["nearai"]
          },
          "services": [
            {
              "id": "claude-sub", "name": "Claude subscription", "kind": "subscription",
              "authenticated": true, "detail": null,
              "requires_consent": true, "consented": false,
              "consent_prompt": {
                "version": 1, "backend_id": "claude-sub", "product": "Claude",
                "summary": "IronWire will read the OAuth token that Claude Code stores on this machine and send requests to api.anthropic.com with it, from this computer only.",
                "points": [
                  "This uses a private authentication path. Anthropic does not document it and may change or block it at any time.",
                  "Using it from a third-party proxy may fall outside your subscription's intended use. If Anthropic objects, it is your account that is affected.",
                  "Your token is never sent anywhere except api.anthropic.com.",
                  "You can use an Anthropic API key instead — fully supported, no ambiguity."
                ],
                "question": "Enable the Claude subscription backend?"
              },
              "connect_command": "ironwire connect claude"
            },
            {
              "id": "nearai", "name": "NEAR AI", "kind": "credits",
              "authenticated": true, "detail": null,
              "requires_consent": false, "consented": true,
              "consent_prompt": null,
              "connect_command": "ironwire connect near"
            }
          ]
        }
        """#
    }
}
