import AppKit
import Foundation
import TCBridge
import TCShellCore

/// Installs and removes the command-line tool: a symlink on the PATH to the
/// CLI inside this app's bundle (`cli_tool` in the core).
///
/// The core reads the link path (unprivileged), decides what is offered,
/// and builds the one AppleScript statement each action runs; this
/// controller passes that statement to `/usr/bin/osascript` as a single
/// argument, so no shell parses anything this app composed. The password
/// prompt is the system's. A cancelled prompt changes nothing and shows
/// nothing; any other failure shows the core's line. After every attempt
/// the link is read again, so the row shows what is there, not what was
/// meant to be.
@MainActor
final class CliToolController: ObservableObject {
    static let shared: CliToolController = {
        let controller = CliToolController()
        controller.observeActivation()
        return controller
    }()

    /// Runs one AppleScript statement with administrator privileges.
    typealias Runner = @Sendable (String) async -> CliToolOutcome

    @Published private(set) var status: CliToolStatus?
    @Published private(set) var busy = false
    /// The link path a change just failed on, for the core's failure line.
    @Published private(set) var failedPath: String?

    private let probe: () -> CliToolStatus?
    private let runner: Runner
    private var activeObserver: NSObjectProtocol?

    init(
        probe: @escaping () -> CliToolStatus? = {
            CliToolStatus.decode(fromJSON: TCCliTool.json(bundlePath: Bundle.main.bundlePath))
        },
        runner: @escaping Runner = CliToolController.runOsascript
    ) {
        self.probe = probe
        self.runner = runner
        status = probe()
    }

    /// Read the link again when the app comes forward: it may have been
    /// changed from a terminal.
    func observeActivation() {
        guard activeObserver == nil else { return }
        activeObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in
            Task { @MainActor in self?.refresh() }
        }
    }

    func refresh() {
        status = probe()
    }

    /// Run `action`'s script, if the core offers it. One at a time.
    func perform(_ action: CliToolStatus.Action) async {
        guard !busy, let script = status?.script(for: action) else { return }
        busy = true
        failedPath = nil
        let outcome = await runner(script)
        refresh()
        if outcome == .failed { failedPath = status?.linkPath }
        busy = false
    }

    /// `/usr/bin/osascript -e <statement>`, off the main thread. The
    /// statement is one argv element; the system shows the password prompt.
    nonisolated static func runOsascript(_ statement: String) async -> CliToolOutcome {
        await withCheckedContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                let process = Process()
                process.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
                process.arguments = ["-e", statement]
                let errors = Pipe()
                process.standardError = errors
                process.standardOutput = FileHandle.nullDevice
                do {
                    try process.run()
                } catch {
                    continuation.resume(returning: .failed)
                    return
                }
                let data = errors.fileHandleForReading.readDataToEndOfFile()
                process.waitUntilExit()
                continuation.resume(
                    returning: CliToolOutcome.classify(
                        exitStatus: process.terminationStatus,
                        standardError: String(decoding: data, as: UTF8.self)))
            }
        }
    }
}
