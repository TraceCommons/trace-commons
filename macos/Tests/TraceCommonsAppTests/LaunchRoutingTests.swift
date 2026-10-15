@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Launch routing after #1242's review (poldsam, 2026-10-05): a refused
/// daemon is a Monitor state, not a first-run one (B1); the launch is quiet
/// (I2, R-44); the Monitor keeps a destination it cannot show (I1); a
/// Settings destination opens Settings alone (M3).
@MainActor
final class LaunchRoutingTests: XCTestCase {
    private static let destinations: [MonitorDestination?] = [
        nil, .inference, .home(.overview), .home(.history), .traces(entryId: nil), .traces(entryId: "entry-7"),
    ]

    /// B1: a daemon that refused to start has no status, so the placeholder
    /// reads as "onboarding required" for an install that finished it long
    /// ago. Every request opens the Monitor at the refusal, whatever that
    /// placeholder says, never first run at Welcome.
    func test_aRefusedDaemonOpensTheMonitorForAnOnboardedPerson() {
        for destination in Self.destinations {
            for requires in [true, false] {
                XCTAssertEqual(
                    LaunchRouting.window(for: destination, startup: .refused("locked"), requiresOnboarding: requires,
                                         onboardingKnown: true),
                    .monitor, "\(String(describing: destination)), requiresOnboarding \(requires)")
                XCTAssertEqual(
                    LaunchRouting.opening(destination, startup: .refused("locked"), requiresOnboarding: requires,
                                          onboardingKnown: true).window,
                    .monitor, "\(String(describing: destination)), requiresOnboarding \(requires)")
            }
        }
        // A daemon that needs its folders still routes to first run, which
        // is where they are collected.
        XCTAssertEqual(LaunchRouting.window(for: nil, startup: .needsRoots, requiresOnboarding: true, onboardingKnown: true),
                       .firstRun)
    }

    /// B1: the Monitor's gate, and a writing Settings section's, draw the
    /// core's refusal for a refused daemon, never the onboarding notice.
    func test_aRefusedDaemonDrawsTheRefusalNotTheOnboardingNotice() {
        let gate = MonitorGate.of(startup: .refused("locked"), onboardingKnown: true, requiresOnboarding: true)
        XCTAssertEqual(gate, .down("locked"))
        XCTAssertEqual(gate.forSettings(availableBeforeOnboarding: false), .down("locked"))
        XCTAssertEqual(gate.forSettings(availableBeforeOnboarding: true), .open)
        XCTAssertEqual(MonitorGate.of(startup: .starting, onboardingKnown: false, requiresOnboarding: true), .awaiting)
        XCTAssertEqual(MonitorGate.of(startup: .running, onboardingKnown: true, requiresOnboarding: true), .signedOut)
        XCTAssertEqual(MonitorGate.of(startup: .needsRoots, onboardingKnown: true, requiresOnboarding: true), .signedOut)
        XCTAssertEqual(MonitorGate.of(startup: .running, onboardingKnown: true, requiresOnboarding: false), .open)
    }

    /// M2: before the core has said, a writing Settings section waits, as
    /// the Monitor's pane does, rather than telling an onboarded person
    /// they are signed out.
    func test_aWritingSettingsSectionWaitsForTheCore() {
        let gate = MonitorGate.of(startup: .running, onboardingKnown: false, requiresOnboarding: true)
        XCTAssertEqual(gate.forSettings(availableBeforeOnboarding: false), .awaiting)
        XCTAssertEqual(gate.forSettings(availableBeforeOnboarding: true), .open)
    }

    /// I2 (owner decision 2026-10-05): no window on an onboarded launch;
    /// first run, activated, while onboarding is required; the Monitor at
    /// the refusal over a refused daemon; and nothing decided from a
    /// status that has not answered, a failed read included.
    func test_theLaunchIsQuiet() {
        XCTAssertEqual(LaunchRouting.launchOpening(startup: .running, statusAnswered: true, requiresOnboarding: false), .nothing)
        XCTAssertEqual(LaunchRouting.launchOpening(startup: .running, statusAnswered: true, requiresOnboarding: true), .firstRun)
        XCTAssertEqual(LaunchRouting.launchOpening(startup: .needsRoots, statusAnswered: false, requiresOnboarding: true), .firstRun)
        XCTAssertEqual(LaunchRouting.launchOpening(startup: .refused("locked"), statusAnswered: false, requiresOnboarding: true),
                       .monitor)
        XCTAssertEqual(LaunchRouting.launchOpening(startup: .starting, statusAnswered: false, requiresOnboarding: true), .wait)
        XCTAssertEqual(LaunchRouting.launchOpening(startup: .running, statusAnswered: false, requiresOnboarding: true), .wait,
                       "a failed first read is not 'onboarding required'")
    }

    /// M3: a Settings destination (a quit refusal, "Manage rules") opens
    /// Settings at its section and no window behind it.
    func test_aSettingsDestinationOpensSettingsAlone() {
        for startup: AppModel.Startup in [.starting, .running, .needsRoots, .refused("locked")] {
            for requires in [true, false] {
                for known in [true, false] {
                    let opening = LaunchRouting.opening(.settings(.compute), startup: startup, requiresOnboarding: requires,
                                                        onboardingKnown: known)
                    XCTAssertNil(opening.window, "\(startup), requires \(requires), known \(known)")
                    XCTAssertEqual(opening.settings, .compute)
                }
            }
        }
        for destination in Self.destinations {
            XCTAssertNotNil(LaunchRouting.opening(destination, startup: .running, requiresOnboarding: false,
                                                  onboardingKnown: true).window)
        }
    }

    /// I1: the Monitor takes only a destination it can show. While
    /// onboarding is required, or before the core has said, Home and
    /// Traces stay pending for first run to hand off.
    func test_theMonitorKeepsADestinationItCannotShow() {
        for destination: MonitorDestination in [.home(.overview), .home(.history), .traces(entryId: nil), .traces(entryId: "entry-7")] {
            XCTAssertFalse(LaunchRouting.monitorConsumes(destination, requiresOnboarding: true, onboardingKnown: true),
                           "\(destination) consumed while onboarding is required")
            XCTAssertFalse(LaunchRouting.monitorConsumes(destination, requiresOnboarding: true, onboardingKnown: false),
                           "\(destination) consumed before the core said")
            XCTAssertTrue(LaunchRouting.monitorConsumes(destination, requiresOnboarding: false, onboardingKnown: true))
        }
        XCTAssertTrue(LaunchRouting.monitorConsumes(.inference, requiresOnboarding: true, onboardingKnown: true),
                      "Inference is shown before onboarding (R-38)")
        XCTAssertTrue(LaunchRouting.monitorConsumes(.inference, requiresOnboarding: true, onboardingKnown: false))
    }

    /// I1: a request leaves its destination for the Monitor. A request with
    /// none (a Dock click, an invite link) does not clear one already
    /// waiting, a Settings destination is the Settings window's, and a
    /// request for the destination already waiting (first run's hand-off
    /// into an open Monitor) is still a request the Monitor sees.
    func test_aRequestLeavesItsDestinationWithoutClearingOne() {
        let navigation = MainWindowNavigation()
        navigation.leave(.traces(entryId: nil))
        XCTAssertEqual(navigation.pending, .traces(entryId: nil))
        XCTAssertEqual(navigation.requests, 1)
        navigation.leave(nil)
        XCTAssertEqual(navigation.pending, .traces(entryId: nil), "a plain request cleared a waiting destination")
        navigation.leave(.settings(.compute))
        XCTAssertEqual(navigation.pending, .traces(entryId: nil), "a Settings destination replaced a Monitor one")
        navigation.leave(.traces(entryId: nil))
        XCTAssertEqual(navigation.requests, 2, "the same destination again must still reach the Monitor")
    }
}
