import Combine
import Foundation
import Sparkle
import TCUpdates

/// Owns Sparkle, or deliberately does not.
///
/// The governing rule is that whoever installed the binary owns replacing
/// it. `UpdatePolicy` decides which of us that is, from a local path check
/// with no network; this type only acts on the answer. Sparkle is
/// constructed at all only in the `.selfUpdating` case -- constructing it
/// and then declining to start it would still leave a framework in the
/// process believing it may schedule work.
@MainActor
final class UpdateController: ObservableObject {
    static let shared = UpdateController()

    @Published private(set) var mode: UpdateMode
    /// When Sparkle last looked. Nil before the first check of this install.
    @Published private(set) var lastCheckDate: Date?
    @Published private(set) var canCheckNow: Bool = false

    /// CFBundleShortVersionString. "unknown" only when running the bare
    /// SwiftPM executable outside a bundle, which is a development case.
    let currentVersion: String

    private var updaterController: SPUStandardUpdaterController?
    private var cancellables = Set<AnyCancellable>()

    init(
        bundle: Bundle = .main,
        homebrew: HomebrewInstallState = HomebrewDetector.detect()
    ) {
        self.currentVersion =
            bundle.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String
            ?? "unknown"
        let feedURL = bundle.object(forInfoDictionaryKey: "SUFeedURL") as? String
        self.mode = UpdatePolicy.mode(homebrew: homebrew, feedURL: feedURL)
    }

    /// Starts the updater if and only if this install is ours.
    ///
    /// Safe to call more than once; the second call is a no-op. Called from
    /// the app's single launch path. Gates on `mode.startsUpdater`, the same
    /// predicate `UpdatePolicyTests` exercises directly, so the mapping from
    /// policy decision to "did Sparkle start" is unit-tested without needing
    /// Sparkle itself.
    func start() {
        guard mode.startsUpdater, updaterController == nil else { return }

        // startingUpdater: true starts the scheduled checks immediately.
        // Everything about WHAT gets checked and how often comes from the
        // Info.plist (SUFeedURL, SUEnableAutomaticChecks,
        // SUAutomaticallyUpdate, SUScheduledCheckInterval) rather than from
        // runtime API calls -- Sparkle's own guidance is that the runtime
        // properties are for responding to user setting changes, not for
        // establishing defaults.
        let controller = SPUStandardUpdaterController(
            startingUpdater: true,
            updaterDelegate: nil,
            userDriverDelegate: nil
        )
        updaterController = controller

        controller.updater.publisher(for: \.canCheckForUpdates)
            .receive(on: DispatchQueue.main)
            .sink { [weak self] value in self?.canCheckNow = value }
            .store(in: &cancellables)

        lastCheckDate = controller.updater.lastUpdateCheckDate
    }

    /// A user-initiated check. Does nothing in any mode but `.selfUpdating`,
    /// because in every other mode there is no updater to ask.
    func checkNow() {
        guard let controller = updaterController else { return }
        controller.updater.checkForUpdates()
        lastCheckDate = controller.updater.lastUpdateCheckDate
    }

    /// Re-reads the last check time. Sparkle updates it on its own schedule,
    /// and a Settings window left open would otherwise show a stale time.
    func refreshLastCheckDate() {
        lastCheckDate = updaterController?.updater.lastUpdateCheckDate
    }
}
