//! Checks that need a real daemon.
//
// Skipped unless `IRONWIRE_LIVE=1`, because they need `ironwire serve` running
// and one of them changes the daemon's pin. Nothing in CI runs them; they are
// here so "does this actually work against the real control API" is a command
// somebody can run rather than a thing somebody remembers to do.
//
//     ironwire serve &
//     IRONWIRE_LIVE=1 swift test --filter LiveDaemonTests
//
// The unit tests above cover the decoding and the rules. These cover the two
// things only a real daemon can show: that the document it actually emits
// matches what this app expects, and that the token dance works against a server
// that really does return 401.

import XCTest

@testable import IronWireKit

@MainActor
final class LiveDaemonTests: XCTestCase {
    override func setUp() async throws {
        try XCTSkipUnless(
            ProcessInfo.processInfo.environment["IRONWIRE_LIVE"] == "1",
            "set IRONWIRE_LIVE=1 with `ironwire serve` running")
    }

    /// The whole contract, against the real thing. A field renamed on the Rust
    /// side shows up here and nowhere else.
    func test_the_running_daemon_returns_a_status_this_app_can_read() async throws {
        let client = ControlClient()
        await client.refresh()

        XCTAssertEqual(client.connection, .connected, "is `ironwire serve` running?")
        let status = try XCTUnwrap(client.status)
        XCTAssertFalse(status.version.isEmpty)
        XCTAssertGreaterThan(status.port, 0)
        // Not an assertion about *this* machine's setup — only that whatever it
        // reported round-tripped into the types the menu draws from.
        for backend in status.backends {
            XCTAssertFalse(backend.id.isEmpty)
            XCTAssertFalse(backend.name.isEmpty)
        }
    }

    /// The acceptance criterion: a 401 re-reads the token file and retries once
    /// before anyone is told anything.
    ///
    /// Set up by pointing a client at a home whose token is wrong, then writing
    /// the real one there before the first call — so the client holds a stale
    /// token in memory while the correct one is on disk, which is exactly what
    /// happens when the daemon is restarted underneath a running app.
    func test_a_stale_token_is_re_read_from_disk_and_retried_once() async throws {
        let real = try XCTUnwrap(Discovery.token(), "no control.token to copy")
        let home = try temporaryHome(token: "not-the-right-token")

        let client = ControlClient(home: home)
        try real.write(to: home.appendingPathComponent("control.token"), atomically: true, encoding: .utf8)

        await client.refresh()
        XCTAssertEqual(client.connection, .connected, "the retry after re-reading the token did not succeed")
        XCTAssertNotNil(client.status)
    }

    /// And when re-reading does not help, it stops — one retry, then say so.
    func test_a_token_that_is_wrong_on_disk_too_reports_an_auth_failure_rather_than_looping() async throws {
        let home = try temporaryHome(token: "not-the-right-token")
        let client = ControlClient(home: home)

        await client.refresh()
        XCTAssertEqual(client.connection, .unauthorised)
        XCTAssertNil(client.status)
    }

    /// Losing the daemon has to *look* like losing the daemon.
    ///
    /// Every figure in a status is an observation with an age the daemon
    /// computed, so a menu redrawn from the last reply keeps saying "observed
    /// 12s ago" for as long as nothing answers. Dropping it is what makes the
    /// icon dim and the menu say "not running" instead of showing yesterday's
    /// numbers in the present tense.
    func test_losing_the_daemon_drops_the_status_rather_than_showing_a_stale_one() async throws {
        let client = ControlClient()
        await client.refresh()
        try XCTSkipUnless(client.connection == .connected, "needs a running daemon")
        XCTAssertNotNil(client.status)

        // Same client, now pointed at a port with nothing on it.
        let orphan = ControlClient(home: try temporaryHome(token: "irrelevant", port: 8399))
        await orphan.refresh()
        XCTAssertNil(orphan.status)
        XCTAssertEqual(orphan.connection, .unreachable)
    }

    /// The ordinary first-launch order: the app is running before the daemon has
    /// ever started, so there is no token to read. It has to notice one appear
    /// without being restarted.
    func test_a_token_that_appears_after_launch_is_picked_up_without_a_restart() async throws {
        let real = try XCTUnwrap(Discovery.token(), "no control.token to copy")
        let home = try temporaryHome()

        // Constructed against an empty home: no token, nothing to send.
        let client = ControlClient(home: home)
        await client.refresh()
        XCTAssertEqual(client.connection, .unreachable)

        // The daemon starts and writes its token.
        try real.write(to: home.appendingPathComponent("control.token"), atomically: true, encoding: .utf8)

        await client.refresh()
        XCTAssertEqual(client.connection, .connected, "the poll loop never re-read the token file")
        XCTAssertNotNil(client.status)
    }

    /// A daemon that is not there is an ordinary state, not an error: no throw,
    /// no invented status.
    func test_a_port_with_no_daemon_on_it_is_reported_as_not_running() async throws {
        let client = ControlClient(home: try temporaryHome(token: "irrelevant", port: 8399))
        await client.refresh()
        XCTAssertEqual(client.connection, .unreachable)
        XCTAssertNil(client.status)
    }

    /// The one write this app makes, round-tripped: the daemon validates it, and
    /// the menu believes the next poll rather than what it asked for.
    func test_a_pin_is_accepted_and_shows_up_in_the_next_status() async throws {
        let client = ControlClient()
        await client.refresh()
        let backends = try XCTUnwrap(client.status?.backends)
        try XCTSkipIf(backends.isEmpty, "no backends configured on this machine")
        let target = backends[0].id

        defer { Task { _ = await client.pin(backend: nil) } }

        let pinned = await client.pin(backend: target)
        guard case .success = pinned else { return XCTFail("pin was rejected: \(pinned)") }
        XCTAssertEqual(client.status?.pin, target)

        let cleared = await client.pin(backend: nil)
        guard case .success = cleared else { return XCTFail("clearing the pin was rejected: \(cleared)") }
        XCTAssertNil(client.status?.pin)
    }

    /// The daemon knows which backends exist and says so in the body. Passing
    /// that through beats "400", which tells the user nothing they can act on.
    func test_pinning_something_that_is_not_a_backend_is_refused_in_words() async throws {
        let client = ControlClient()
        let outcome = await client.pin(backend: "not-a-real-backend")
        guard case .failure(let error) = outcome else {
            return XCTFail("the daemon accepted a backend that does not exist")
        }
        XCTAssertTrue(error.message.contains("not-a-real-backend"), error.message)
    }

    // MARK: - Settings

    /// The settings contract, against the real thing. A field renamed on the
    /// Rust side shows up here and nowhere else.
    func test_the_running_daemon_returns_settings_this_app_can_read() async throws {
        let client = ControlClient()
        let fetched = await client.refreshSettings()
        XCTAssertTrue(fetched, "is `ironwire serve` running?")

        let settings = try XCTUnwrap(client.settings)
        XCTAssertEqual(
            settings.privacy.options.map(\.id),
            ["off", "credentials", "pii", "full"],
            "the ladder, in order")
        XCTAssertFalse(settings.privacy.summary.isEmpty)
    }

    /// The half of the contract the pane is built on: a mode it must grey out
    /// always arrives with the sentence it greys out *with*, and a mode it can
    /// offer never carries one.
    ///
    /// What is deliberately not asserted is *when* a mode is blocked. That rule
    /// is `full_is_blocked` in `control.rs` and it is more than "is anything
    /// trusted" — `trusted_backends` defaults to `["nearai"]`, a backend that
    /// registers whether or not a key was found, so a named destination is not
    /// a usable one. A copy of that condition here is exactly the second
    /// implementation this app exists not to have, and it was wrong within one
    /// release of being written.
    func test_a_privacy_mode_that_cannot_be_picked_arrives_with_the_reason() async throws {
        let client = ControlClient()
        _ = await client.refreshSettings()
        let settings = try XCTUnwrap(client.settings)
        XCTAssertFalse(settings.privacy.options.isEmpty, "no ladder to render")

        for option in settings.privacy.options {
            if option.selectable {
                XCTAssertNil(
                    option.unavailableBecause,
                    "\(option.id) can be picked and still explains why it cannot")
            } else {
                let because = try XCTUnwrap(
                    option.unavailableBecause, "\(option.id) is greyed out with nothing to show")
                XCTAssertFalse(because.isEmpty)
            }
        }
    }

    /// The round trip that matters: switch, observe it in force, put it back.
    func test_a_privacy_mode_change_takes_effect_and_can_be_undone() async throws {
        let client = ControlClient()
        _ = await client.refreshSettings()
        let original = try XCTUnwrap(client.settings?.privacy.mode)

        let target = original == "off" ? "credentials" : "off"
        guard case .success = await client.setPrivacyMode(target) else {
            return XCTFail("the daemon refused a mode it listed as selectable")
        }
        XCTAssertEqual(client.settings?.privacy.mode, target, "not in force after the write")
        // The status surface has to agree — it is what the menu draws from.
        XCTAssertNotNil(client.status)

        guard case .success = await client.setPrivacyMode(original) else {
            return XCTFail("could not restore the original mode")
        }
        XCTAssertEqual(client.settings?.privacy.mode, original)
    }

    func test_a_mode_that_does_not_exist_is_refused_in_words() async throws {
        let client = ControlClient()
        let outcome = await client.setPrivacyMode("maximum")
        guard case .failure(let error) = outcome else {
            return XCTFail("the daemon accepted a mode that does not exist")
        }
        XCTAssertTrue(error.message.contains("maximum"), error.message)
    }

    /// Consent, end to end, at the version the daemon is currently asking at —
    /// and then withdrawn again, because this test must not leave a subscription
    /// enabled on somebody's machine.
    func test_consent_can_be_granted_and_withdrawn_at_the_current_prompt_version() async throws {
        let client = ControlClient()
        _ = await client.refreshSettings()
        let settings = try XCTUnwrap(client.settings)
        guard let service = settings.services.first(where: { $0.canToggle && !$0.consented }),
              let prompt = service.consentPrompt
        else {
            throw XCTSkip("no subscription backend is awaiting consent on this machine")
        }
        XCTAssertTrue(prompt.isComplete, "the prompt has to be presentable before it is answered")

        guard case .success = await client.setConsent(
            backend: service.id, granted: true, promptVersion: prompt.version, binding: prompt.binding)
        else { return XCTFail("the daemon refused consent at its own current version") }

        let after = try XCTUnwrap(client.settings?.services.first { $0.id == service.id })
        XCTAssertTrue(after.consented)

        guard case .success = await client.setConsent(
            backend: service.id, granted: false, promptVersion: prompt.version)
        else { return XCTFail("could not withdraw consent again") }
        let restored = try XCTUnwrap(client.settings?.services.first { $0.id == service.id })
        XCTAssertFalse(restored.consented, "this test must not leave a subscription enabled")
    }

    /// The whole point of sending the version: an answer belongs to the wording
    /// it answered, and a stale client must not be able to grant on the strength
    /// of a question nobody read.
    func test_consent_at_a_version_the_daemon_is_no_longer_asking_is_refused() async throws {
        let client = ControlClient()
        _ = await client.refreshSettings()
        let settings = try XCTUnwrap(client.settings)
        guard let service = settings.services.first(where: { $0.canToggle }) else {
            throw XCTSkip("no subscription backend on this machine")
        }

        let outcome = await client.setConsent(backend: service.id, granted: true, promptVersion: 0)
        guard case .failure(let error) = outcome else {
            return XCTFail("the daemon accepted consent to a version it is not asking")
        }
        XCTAssertTrue(error.message.contains("different question"), error.message)
    }

    private func temporaryHome(token: String? = nil, port: Int? = nil) throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("ironwire-live-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        if let token {
            try token.write(to: dir.appendingPathComponent("control.token"), atomically: true, encoding: .utf8)
        }
        if let port {
            try "[server]\nport = \(port)\n".write(
                to: dir.appendingPathComponent("config.toml"), atomically: true, encoding: .utf8)
        }
        addTeardownBlock { try? FileManager.default.removeItem(at: dir) }
        return dir
    }
}
