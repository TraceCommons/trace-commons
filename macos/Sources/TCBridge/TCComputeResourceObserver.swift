import AppKit
import Darwin
import IOKit.ps

/// Platform facts only. Rust owns eligibility, stop urgency, consent and copy.
public struct TCComputeResourceReading: Encodable, Equatable, Sendable {
    public enum Power: String, Encodable, Sendable { case ac, battery, ups, unknown }
    public enum Thermal: String, Encodable, Sendable { case nominal, fair, serious, critical, unknown }
    public enum Memory: String, Encodable, Sendable { case normal, warning, critical, unknown }
    public var power: Power
    public var lowPowerMode: Bool?
    public var thermal: Thermal
    public var memory: Memory

    enum CodingKeys: String, CodingKey {
        case power, lowPowerMode = "low_power_mode", thermal, memory
    }

    public init(power: Power, lowPowerMode: Bool?, thermal: Thermal, memory: Memory) {
        self.power = power
        self.lowPowerMode = lowPowerMode
        self.thermal = thermal
        self.memory = memory
    }

    public static let unknown = Self(power: .unknown, lowPowerMode: nil, thermal: .unknown, memory: .unknown)

    public func encode(to encoder: any Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(power, forKey: .power)
        try values.encode(lowPowerMode, forKey: .lowPowerMode)
        try values.encode(thermal, forKey: .thermal)
        try values.encode(memory, forKey: .memory)
    }

    /// A new OS query for every field, including memory bootstrap. A denied or
    /// unsupported sysctl is unknown; dispatch silence is never normal memory.
    public static func readCurrent() -> Self {
        let power: Power
        if let snapshot = IOPSCopyPowerSourcesInfo()?.takeRetainedValue(),
           let source = IOPSGetProvidingPowerSourceType(snapshot)?.takeUnretainedValue() {
            power = decodePower(source as String)
        } else {
            power = .unknown
        }
        let thermal: Thermal
        switch ProcessInfo.processInfo.thermalState {
        case .nominal: thermal = .nominal
        case .fair: thermal = .fair
        case .serious: thermal = .serious
        case .critical: thermal = .critical
        @unknown default: thermal = .unknown
        }
        var pressure: UInt32 = 0
        var size = MemoryLayout<UInt32>.size
        let result = sysctlbyname("kern.memorystatus_vm_pressure_level", &pressure, &size, nil, 0)
        return Self(power: power, lowPowerMode: ProcessInfo.processInfo.isLowPowerModeEnabled,
                    thermal: thermal, memory: decodeMemory(result == 0 && size == MemoryLayout<UInt32>.size ? pressure : nil))
    }

    static func decodePower(_ value: String?) -> Power {
        switch value {
        case kIOPMACPowerKey: return .ac
        case kIOPMBatteryPowerKey: return .battery
        case kIOPMUPSPowerKey: return .ups
        default: return .unknown
        }
    }

    static func decodeMemory(_ value: UInt32?) -> Memory {
        // XNU sysctl_memorystatus_vm_pressure_level converts internal levels
        // into DISPATCH_MEMORYPRESSURE_* flags. Internal enum 0/1/2/3 is NOT
        // this ABI. Reject combinations/new values instead of guessing normal.
        // Apple source (read-only, masked sysctl; may be denied/removed):
        // https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/kern/kern_memorystatus_notify.c#L1885
        switch value {
        case UInt32(DispatchSource.MemoryPressureEvent.normal.rawValue): return .normal
        case UInt32(DispatchSource.MemoryPressureEvent.warning.rawValue): return .warning
        case UInt32(DispatchSource.MemoryPressureEvent.critical.rawValue): return .critical
        default: return .unknown
        }
    }
}

/// App-owned, not window-owned. All delivery is synchronous on the main actor;
/// no Task hop can reorder sleep and wake. Call stop before freeing the FFI host.
/// The controller issues the opaque ticket BEFORE the read, and independently
/// expires it: delayed readings must never receive a fresh timestamp on submit.
@MainActor
public final class TCComputeResourceObserver {
    enum Event: Sendable { case refresh, memory(TCComputeResourceReading.Memory), sleep, wake }
    typealias Cancel = @MainActor @Sendable () -> Void
    typealias Install = (@escaping @MainActor @Sendable (Event) -> Void) -> Cancel
    private let begin: () -> String?
    private let submit: (String, TCComputeResourceReading) -> Void
    private let onSleep: () -> Void
    private let onWake: () -> Void
    private let read: () -> TCComputeResourceReading
    private let install: Install
    private var cancel: Cancel?
    private var running = false
    private var sleeping = false
    private var generation: UInt64 = 0

    public convenience init(begin: @escaping () -> String?,
                            submit: @escaping (String, TCComputeResourceReading) -> Void,
                            sleep: @escaping () -> Void, wake: @escaping () -> Void) {
        self.init(begin: begin, submit: submit, sleep: sleep, wake: wake,
                  read: TCComputeResourceReading.readCurrent, install: Self.installNative)
    }

    init(begin: @escaping () -> String?, submit: @escaping (String, TCComputeResourceReading) -> Void,
         sleep: @escaping () -> Void, wake: @escaping () -> Void,
         read: @escaping () -> TCComputeResourceReading, install: @escaping Install) {
        self.begin = begin
        self.submit = submit
        self.onSleep = sleep
        self.onWake = wake
        self.read = read
        self.install = install
    }

    public func start() {
        guard !running else { return }
        running = true
        let current = generation
        cancel = install { [weak self] event in
            guard let self, self.running, self.generation == current else { return }
            self.receive(event)
        }
        refresh()
    }

    public func stop() {
        guard running else { return }
        running = false
        generation &+= 1
        cancel?()
        cancel = nil
        // Do not clear sleeping here: stop/start is not evidence of wake.
    }

    deinit {
        if let cancel { Task { @MainActor in cancel() } }
    }

    private func receive(_ event: Event) {
        switch event {
        case .refresh: refresh()
        case .memory(let level): refresh(memoryEvent: level)
        case .sleep:
            sleeping = true
            onSleep()
        case .wake:
            onWake() // Invalidate Rust observations/telemetry before new reads.
            sleeping = false
            refresh() // Never Resume: only explicit user intent can restart.
        }
    }

    private func refresh(memoryEvent: TCComputeResourceReading.Memory? = nil) {
        guard running, !sleeping, let ticket = begin() else { return }
        let current = generation
        var reading = read()
        // The delivered event is new evidence, not cached telemetry. A brief
        // critical event still reaches the reducer even if the query recovered.
        if memoryEvent == .critical {
            reading.memory = .critical
        } else if memoryEvent == .warning, reading.memory == .normal {
            reading.memory = .warning
        }
        guard running, !sleeping, generation == current else { return }
        submit(ticket, reading)
    }

    private static func installNative(_ receive: @escaping @MainActor @Sendable (Event) -> Void) -> Cancel {
        let timer = DispatchSource.makeTimerSource(queue: .main)
        timer.schedule(deadline: .now() + 2, repeating: 2, leeway: .milliseconds(100))
        timer.setEventHandler { MainActor.assumeIsolated { receive(.refresh) } }
        timer.resume()

        let memory = DispatchSource.makeMemoryPressureSource(eventMask: [.normal, .warning, .critical], queue: .main)
        memory.setEventHandler { [weak memory] in
            guard let flags = memory?.data else { return }
            let level: TCComputeResourceReading.Memory = flags.contains(.critical) ? .critical
                : flags.contains(.warning) ? .warning : .normal
            MainActor.assumeIsolated { receive(.memory(level)) }
        }
        memory.resume()

        let center = NotificationCenter.default
        let thermal = center.addObserver(forName: ProcessInfo.thermalStateDidChangeNotification, object: nil, queue: .main) { _ in
            MainActor.assumeIsolated { receive(.refresh) }
        }
        let lowPower = center.addObserver(forName: .NSProcessInfoPowerStateDidChange, object: nil, queue: .main) { _ in
            MainActor.assumeIsolated { receive(.refresh) }
        }
        let workspace = NSWorkspace.shared.notificationCenter
        let sleep = workspace.addObserver(forName: NSWorkspace.willSleepNotification, object: nil, queue: .main) { _ in
            MainActor.assumeIsolated { receive(.sleep) }
        }
        let wake = workspace.addObserver(forName: NSWorkspace.didWakeNotification, object: nil, queue: .main) { _ in
            MainActor.assumeIsolated { receive(.wake) }
        }
        let context = PowerCallback { receive(.refresh) }
        let powerSource = IOPSNotificationCreateRunLoopSource({ pointer in
            guard let pointer else { return }
            MainActor.assumeIsolated {
                Unmanaged<PowerCallback>.fromOpaque(pointer).takeUnretainedValue().receive()
            }
        }, Unmanaged.passUnretained(context).toOpaque())?.takeRetainedValue()
        if let powerSource { CFRunLoopAddSource(CFRunLoopGetMain(), powerSource, .commonModes) }

        return {
            timer.cancel()
            memory.cancel()
            center.removeObserver(thermal)
            center.removeObserver(lowPower)
            workspace.removeObserver(sleep)
            workspace.removeObserver(wake)
            if let powerSource {
                CFRunLoopRemoveSource(CFRunLoopGetMain(), powerSource, .commonModes)
                CFRunLoopSourceInvalidate(powerSource)
            }
            withExtendedLifetime(context) {} // Outlive the registered raw pointer.
        }
    }
}

@MainActor
private final class PowerCallback {
    let receive: () -> Void
    init(_ receive: @escaping () -> Void) { self.receive = receive }
}
