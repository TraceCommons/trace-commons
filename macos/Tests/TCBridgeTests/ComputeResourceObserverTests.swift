import Foundation
import Testing
@testable import TCBridge

@Suite("Compute resource observer")
@MainActor
struct ComputeResourceObserverTests {
    @MainActor
    final class Harness {
        var log: [String] = []
        var samples: [TCComputeResourceReading] = []
        var callbacks: [@MainActor @Sendable (TCComputeResourceObserver.Event) -> Void] = []
        var reading = TCComputeResourceReading(power: .ac, lowPowerMode: false, thermal: .nominal, memory: .normal)
        var ticket: String? = "opaque-ticket"
        lazy var observer = TCComputeResourceObserver(
            begin: { [unowned self] in log.append("begin"); return ticket },
            submit: { [unowned self] ticket, reading in
                #expect(ticket == "opaque-ticket")
                log.append("submit"); samples.append(reading)
            }, sleep: { [unowned self] in log.append("sleep") },
            wake: { [unowned self] in log.append("wake") },
            read: { [unowned self] in log.append("read"); return reading },
            install: { [unowned self] callback in
                log.append("install"); callbacks.append(callback)
                return { [weak self] in self?.log.append("cancel") }
            })
    }

    @Test func ticketPrecedesEveryReadAndNoCachedValuesAreRestamped() {
        let h = Harness()
        h.observer.start()
        #expect(h.log == ["install", "begin", "read", "submit"])
        h.reading = .unknown
        h.callbacks[0](.refresh)
        #expect(h.samples.last == .unknown)
        #expect(h.log.suffix(3) == ["begin", "read", "submit"])
        h.observer.stop()
    }

    @Test func startIsIdempotentAndOldCallbacksCannotRestartStoppedObserver() {
        let h = Harness()
        h.observer.start()
        h.observer.start()
        #expect(h.callbacks.count == 1)
        h.observer.stop()
        h.observer.stop()
        h.callbacks[0](.refresh)
        h.callbacks[0](.wake)
        #expect(h.samples.count == 1)
        #expect(h.log.filter { $0 == "cancel" }.count == 1)
        h.observer.start()
        h.callbacks[0](.sleep)
        h.callbacks[0](.refresh)
        #expect(!h.log.contains("sleep"))
        h.callbacks[1](.refresh)
        #expect(h.samples.count == 3)
        h.observer.stop()
    }

    @Test func sleepSuppressesReadsAndWakeInvalidatesBeforeFreshRead() {
        let h = Harness()
        h.observer.start()
        h.callbacks[0](.sleep)
        h.callbacks[0](.refresh)
        #expect(h.samples.count == 1)
        h.callbacks[0](.wake)
        #expect(h.log.suffix(4) == ["wake", "begin", "read", "submit"])
        h.callbacks[0](.sleep)
        h.observer.stop()
        h.observer.start()
        #expect(h.samples.count == 2) // Restart is not wake evidence.
        h.callbacks[1](.wake)
        #expect(h.samples.count == 3)
        h.observer.stop()
    }

    @Test func hostRebindingPreservesSleepAndContinuousRegistration() {
        let h = Harness()
        h.observer.start()
        h.callbacks[0](.sleep)
        h.observer.hostDidChange()
        #expect(h.log.suffix(2) == ["sleep", "sleep"])
        #expect(h.samples.count == 1)
        #expect(h.callbacks.count == 1)
        h.callbacks[0](.refresh)
        #expect(h.samples.count == 1)
        h.callbacks[0](.wake)
        #expect(h.log.suffix(4) == ["wake", "begin", "read", "submit"])
        h.observer.hostDidChange()
        #expect(h.log.suffix(4) == ["wake", "begin", "read", "submit"])
        #expect(h.callbacks.count == 1)
        h.callbacks[0](.refresh)
        #expect(h.samples.count == 4) // The original callback remains live.
        #expect(h.log.allSatisfy { ["install", "begin", "read", "submit", "sleep", "wake"].contains($0) })
        h.observer.stop()
        let stoppedLog = h.log
        h.observer.hostDidChange()
        #expect(h.log == stoppedLog)
    }

    @Test func failedBeginDoesNotReadOrSubmit() {
        let h = Harness()
        h.ticket = nil
        h.observer.start()
        h.callbacks[0](.refresh)
        #expect(h.log == ["install", "begin", "begin"])
        #expect(h.samples.isEmpty)
        h.observer.stop()
    }

    @Test func transientMemoryEventsCannotBeErasedByRecovery() {
        let h = Harness()
        h.observer.start()
        h.callbacks[0](.memory(.critical))
        #expect(h.samples.last?.memory == .critical)
        h.callbacks[0](.refresh)
        #expect(h.samples.last?.memory == .normal)
        h.reading.memory = .unknown
        h.callbacks[0](.memory(.normal))
        #expect(h.samples.last?.memory == .unknown)
        h.callbacks[0](.memory(.warning))
        #expect(h.samples.last?.memory == .unknown)
        h.observer.stop()
    }

    @Test func powerAndMemoryMappingsRejectUnexpectedValues() {
        #expect(TCComputeResourceReading.decodePower("AC Power") == .ac)
        #expect(TCComputeResourceReading.decodePower("Battery Power") == .battery)
        #expect(TCComputeResourceReading.decodePower("UPS Power") == .ups)
        #expect(TCComputeResourceReading.decodePower(nil) == .unknown)
        #expect(TCComputeResourceReading.decodePower("new-source") == .unknown)
        #expect(TCComputeResourceReading.decodeMemory(1) == .normal)
        #expect(TCComputeResourceReading.decodeMemory(2) == .warning)
        #expect(TCComputeResourceReading.decodeMemory(4) == .critical)
        for value: UInt32? in [nil, 0, 3, 5, 8, UInt32.max] {
            #expect(TCComputeResourceReading.decodeMemory(value) == .unknown)
        }
    }

    @Test func unknownLowPowerIsExplicitJSONNull() throws {
        let data = try JSONEncoder().encode(TCComputeResourceReading.unknown)
        let object = try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
        #expect(object["low_power_mode"] is NSNull)
        #expect(object["power"] as? String == "unknown")
        #expect(object.count == 4)
    }
}
