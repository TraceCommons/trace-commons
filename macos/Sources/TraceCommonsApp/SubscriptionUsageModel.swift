import Foundation
import Observation
import TCShellCore

/// The Insights "Subscriptions" card's reading (`subscription_usage`).
///
/// Reads the daemon's held snapshot when the Overview opens and never asks
/// for a probe by itself: only Refresh sends `refresh: true` (design 3.3 and
/// the 2026-10 slice decision, no background polling). After a Refresh the
/// card reads the snapshot again on a short schedule until no login says
/// `refreshing`. The daemon's probe has a 20-second budget, then up to 4
/// seconds to stop Codex (2 after closing stdin, 2 after SIGTERM), on top
/// of finding Codex first; the schedule runs past all of that and stops.
@Observable @MainActor
final class SubscriptionUsageModel {
    /// One `subscription_usage` read; the argument is `refresh`.
    typealias Reader = @Sendable (Bool) async throws -> DaemonData.SubscriptionUsage
    typealias Pause = @Sendable (Duration) async -> Void

    /// The waits between re-reads after a Refresh, 35 seconds in all.
    static let pollSchedule: [Duration] = [
        .seconds(1), .seconds(2), .seconds(2), .seconds(3), .seconds(5), .seconds(5), .seconds(5),
        .seconds(4), .seconds(4), .seconds(4),
    ]

    private let reader: Reader?
    private let pause: Pause
    private var pollTask: Task<Void, Never>?
    private var generation = UUID()

    /// The last answer read. Kept through a failed read, so a transient
    /// failure does not take the card away; `nil` once the daemon said it
    /// does not serve the method.
    private(set) var usage: DaemonData.SubscriptionUsage?
    /// A Refresh is being asked for and followed.
    private(set) var asking = false

    init(reader: Reader?, pause: @escaping Pause = { try? await Task.sleep(for: $0) }) {
        self.reader = reader
        self.pause = pause
    }

    /// The card is drawn only while the switch is on: an answer of
    /// `enabled: false`, an older daemon's `unknown_method` and no answer
    /// yet all draw nothing.
    var shown: Bool { usage?.enabled == true }

    /// The daemon is probing, or a Refresh is being followed.
    var refreshing: Bool {
        asking || usage?.logins?.contains { $0.refreshing == true } == true
    }

    func open() {
        let generation = generation
        Task { await self.read(refresh: false, generation: generation) }
    }

    func close() {
        pollTask?.cancel()
        pollTask = nil
        asking = false
        generation = UUID()
    }

    /// Refresh: ask for a probe, then follow the snapshot until the probe
    /// is done or the schedule runs out. A second press while one is being
    /// followed is ignored; the daemon's 60-second floor would refuse it.
    func refresh() {
        guard !asking, reader != nil else { return }
        asking = true
        let generation = generation
        pollTask = Task {
            await self.follow(generation: generation)
            if self.generation == generation { self.asking = false }
        }
    }

    /// For tests: await the Refresh in flight.
    func waitForRefresh() async { await pollTask?.value }

    private func follow(generation: UUID) async {
        await read(refresh: true, generation: generation)
        for wait in Self.pollSchedule {
            guard self.generation == generation, !Task.isCancelled,
                  usage?.logins?.contains(where: { $0.refreshing == true }) == true else { return }
            await pause(wait)
            guard !Task.isCancelled else { return }
            await read(refresh: false, generation: generation)
        }
    }

    private func read(refresh: Bool, generation: UUID) async {
        guard let reader else { return }
        do {
            let answer = try await reader(refresh)
            guard self.generation == generation else { return }
            usage = answer
        } catch let error as DaemonDataError where error.isNotServed {
            guard self.generation == generation else { return }
            usage = nil
        } catch {
            // Keep what is held: the next read or Refresh tries again.
        }
    }
}
