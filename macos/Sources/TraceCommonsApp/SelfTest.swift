import AppKit
import Foundation

/// Drives the typed layer against the live in-process daemon and writes what
/// came back to `TRACE_COMMONS_SELFTEST_OUT`.
///
/// A menu-bar app has no stdout worth reading, and a screenshot cannot prove
/// that `approve` then `cancel` actually round-trips an entry back to
/// `pending`. This does, in text, against the same daemon the UI is driving.
/// It is inert unless the environment variable is set.
enum SelfTest {
    @MainActor
    static func runIfRequested(model: AppModel) {
        guard let path = ProcessInfo.processInfo.environment["TRACE_COMMONS_SELFTEST_OUT"],
              !path.isEmpty
        else { return }
        Task { @MainActor in
            try? await Task.sleep(nanoseconds: 12_000_000_000)
            let report = await run(model: model)
            try? report.write(toFile: path, atomically: true, encoding: .utf8)
            NSLog("trace-commons: self-test written to \(path)")
        }
    }

    @MainActor
    private static func run(model: AppModel) async -> String {
        var lines: [String] = ["trace-commons macOS shell self-test"]

        lines.append("startup: \(model.startup)")
        lines.append("status.logged_in: \(model.status.loggedIn)")
        lines.append("status.paused: \(model.status.paused)")
        lines.append("status.queue_depth: \(model.status.queueDepth)")
        lines.append("status.health: \(model.status.health.lastErrorLabel ?? "none")")
        lines.append("health copy: \(model.health?.title ?? "(healthy)")")
        lines.append("decisions owed (badge): \(model.decisionsOwed)")
        for row in model.waitingByProject {
            lines.append("  waiting: \(row.label) count=\(row.count) bytes=\(row.bytes)")
        }
        lines.append("consent options: \(model.consentScopes.map(\.name).joined(separator: ", "))")

        guard let entry = model.awaitingDecision.first else {
            lines.append("FAIL: nothing pending, so nothing else can be exercised")
            return lines.joined(separator: "\n") + "\n"
        }
        lines.append("entry: project=\(entry.projectLabel) agent=\(entry.agentName) state=\(entry.state)")

        if let summary = model.summaries[entry.entryID] {
            lines.append("socket preview: would_send=\(summary.wouldSendBytes) "
                + "raw=\(summary.rawSessionBytes) events=\(summary.eventCount)")
            lines.append("redaction receipt: \(summary.redactionReceipt)")
            lines.append("opening prompt: \(summary.openingPrompt.prefix(80))")
        } else {
            lines.append("socket preview: not loaded (\(model.summaryErrors[entry.entryID] ?? "pending"))")
        }

        // In-process preview plus two searches: one that must hit, one that
        // must not. This is the affordance the whole review moment rests on.
        switch await model.openPreview(entryID: entry.entryID) {
        case .opened(let preview):
            lines.append("ffi preview body bytes: \(preview.body.utf8.count)")
            let hit = preview.search("Northwind") ?? []
            let miss = preview.search("this-string-is-not-in-the-trace") ?? []
            lines.append("search Northwind -> \(hit.count) match(es) at byte offsets \(hit)")
            lines.append("search absent-string -> \(miss.count) match(es)")
            lines.append("redacted body contains raw AWS key: "
                + "\(preview.body.contains("AKIAIOSFODNN7EXAMPLE"))")
            lines.append("redacted body contains raw GitHub token: "
                + "\(preview.body.contains("ghp_abcdefghijklmnop0123456789"))")
            preview.close()
        case .failed(let message):
            lines.append("FAIL: preview -> \(message)")
        }

        // approve -> cancel: the five-second undo, exercised for real.
        model.approve(entry)
        try? await Task.sleep(nanoseconds: 1_500_000_000)
        lines.append("after approve: undo=\(model.undo != nil) "
            + "still-awaiting=\(model.awaitingDecision.contains(where: { $0.entryID == entry.entryID }))")
        model.undoApproval()
        try? await Task.sleep(nanoseconds: 1_500_000_000)
        lines.append("after undo: back-in-pending="
            + "\(model.awaitingDecision.contains(where: { $0.entryID == entry.entryID }))")

        // pause -> resume.
        model.pause(until: nil)
        try? await Task.sleep(nanoseconds: 1_000_000_000)
        lines.append("after pause: paused=\(model.status.paused)")
        model.resume()
        try? await Task.sleep(nanoseconds: 1_000_000_000)
        lines.append("after resume: paused=\(model.status.paused)")

        lines.append("last action error: \(model.lastActionError ?? "none")")
        return lines.joined(separator: "\n") + "\n"
    }
}
