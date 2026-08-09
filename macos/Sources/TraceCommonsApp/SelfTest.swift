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
        if let path = ProcessInfo.processInfo.environment["TRACE_COMMONS_SELFTEST_OUT"],
           !path.isEmpty
        {
            Task { @MainActor in
                try? await Task.sleep(nanoseconds: 12_000_000_000)
                let report = await run(model: model)
                try? report.write(toFile: path, atomically: true, encoding: .utf8)
                NSLog("trace-commons: self-test written to \(path)")
            }
        }
        runOnboardingSelfTestIfRequested(model: model)
        runResumeCheckIfRequested(model: model)
    }

    /// Reports the exact two predicates `MainWindowView` branches on
    /// (`status.logged_in`, `isOnboardingComplete`), with no side effects.
    /// Meant to be run as a *second*, separate launch against a state
    /// directory a prior `TRACE_COMMONS_ONBOARD_SELFTEST_OUT` run already
    /// enrolled -- proving what a relaunch after a mid-onboarding quit
    /// actually resumes to, from a fresh `AppModel`/`UserDefaults` read
    /// rather than in-memory state carried over from the same process.
    @MainActor
    private static func runResumeCheckIfRequested(model: AppModel) {
        guard let path = ProcessInfo.processInfo.environment["TRACE_COMMONS_RESUME_CHECK_OUT"],
              !path.isEmpty
        else { return }
        Task { @MainActor in
            try? await Task.sleep(nanoseconds: 3_000_000_000)
            model.refreshStatus()
            try? await Task.sleep(nanoseconds: 1_500_000_000)
            let report = "trace-commons macOS resume check\n"
                + "status.logged_in=\(model.status.loggedIn) "
                + "tenant_id=\(model.status.tenantID ?? "nil") "
                + "consent_scopes=\(model.status.consentScopes)\n"
                + "isOnboardingComplete=\(model.isOnboardingComplete)\n"
                + "would show: "
                + (!model.status.loggedIn || !model.isOnboardingComplete
                    ? "onboarding (resumed at .consent, since logged_in is true)"
                    : "main window") + "\n"
            try? report.write(toFile: path, atomically: true, encoding: .utf8)
            NSLog("trace-commons: resume check written to \(path)")
        }
    }

    /// Drives the same `AppModel` calls `OnboardingCoordinatorView` makes,
    /// in the same order, against a real (not mocked) daemon and a real
    /// network issuer -- see
    /// `docs/superpowers/plans/macos-onboarding-flow-report.md` for why this
    /// exists: proving the six-screen chain persists what a contributor
    /// chose needs a real `enroll` round trip against a stub issuer, and
    /// this repo has no macOS GUI-automation tool to drive
    /// `OnboardingConnectView`'s text field and buttons directly. This
    /// exercises the identical calls in the identical order instead:
    /// `enroll(invite:)` with no scopes (screen 2), then
    /// `setConsentScopes` with a chosen scope set (screen 3), then
    /// `acknowledgeNearAINotice()` when the operator has the second scanner
    /// configured (screen 4) -- and reports what the daemon's own `status`
    /// says afterward, which is the only thing this self-test asserts on.
    @MainActor
    private static func runOnboardingSelfTestIfRequested(model: AppModel) {
        guard let path = ProcessInfo.processInfo.environment["TRACE_COMMONS_ONBOARD_SELFTEST_OUT"],
              !path.isEmpty,
              let invite = ProcessInfo.processInfo.environment["TRACE_COMMONS_ONBOARD_SELFTEST_INVITE"],
              !invite.isEmpty
        else { return }
        Task { @MainActor in
            try? await Task.sleep(nanoseconds: 3_000_000_000)
            let report = await runOnboarding(model: model, invite: invite)
            try? report.write(toFile: path, atomically: true, encoding: .utf8)
            NSLog("trace-commons: onboarding self-test written to \(path)")
        }
    }

    @MainActor
    private static func runOnboarding(model: AppModel, invite: String) async -> String {
        var lines: [String] = ["trace-commons macOS onboarding self-test"]
        lines.append("before: status.logged_in=\(model.status.loggedIn) "
            + "consent_scopes=\(model.status.consentScopes)")

        // Screen 2: enroll with no scopes -- matches
        // `OnboardingConnectContent.join`, which never passes `scopes`.
        switch await model.enroll(invite: invite) {
        case .succeeded(let result):
            lines.append("enroll: enrolled=\(result.enrolled) "
                + "tenant_id=\(result.tenantID ?? "nil") "
                + "consent_scopes=\(result.consentScopes ?? [])")
        case .failed:
            lines.append("FAIL: enroll did not succeed")
            return lines.joined(separator: "\n") + "\n"
        }

        // Screen 3: apply the chosen scopes -- matches
        // `OnboardingCoordinatorView.advanceFromConsent`: always-on scopes
        // plus whatever was "ticked" (hardcoded here to the two scopes a
        // real contributor would pick from `consent_options`).
        let alwaysOn = model.consentScopes.filter(\.alwaysOn).map(\.name)
        let chosen = Set(alwaysOn).union(["public_attribution", "benchmark_only"])
        switch await model.setConsentScopes(Array(chosen)) {
        case .succeeded(let scopes):
            lines.append("set_consent_scopes: consent_scopes=\(scopes)")
        case .failed:
            lines.append("FAIL: set_consent_scopes did not succeed")
            return lines.joined(separator: "\n") + "\n"
        }

        // Screen 4: only when the operator has the second scanner
        // configured -- matches `OnboardingPrivacyScanContent.continueButton`
        // gated on `nearAIConfigured`.
        if model.daemonSettings?.nearAIConfigured == true {
            model.acknowledgeNearAINotice()
            try? await Task.sleep(nanoseconds: 1_500_000_000)
            lines.append("acknowledge_near_ai_notice: health=\(model.status.health.lastErrorLabel ?? "none")")
        } else {
            lines.append("near_ai not configured; screen 4 would not have shown")
        }

        model.refreshStatus()
        try? await Task.sleep(nanoseconds: 1_500_000_000)
        lines.append("after: status.logged_in=\(model.status.loggedIn) "
            + "tenant_id=\(model.status.tenantID ?? "nil") "
            + "consent_scopes=\(model.status.consentScopes)")

        // Screen 6 equivalent: mark onboarding complete, the way
        // `OnboardingDoneView`'s `onFinish` does via `MainWindowView`'s
        // `onComplete` closure, and confirm it sticks -- unless asked to
        // stop short of it, which is how this same self-test doubles as
        // the "resumed mid-onboarding across a relaunch" check: a second
        // launch against the same state directory, without this env var,
        // reads `status.logged_in`/`isOnboardingComplete` fresh and proves
        // whether the marker survived (it must not have, here).
        if ProcessInfo.processInfo.environment["TRACE_COMMONS_ONBOARD_SELFTEST_SKIP_COMPLETE"] == "1" {
            lines.append("skipped markOnboardingComplete (TRACE_COMMONS_ONBOARD_SELFTEST_SKIP_COMPLETE=1)")
        } else {
            model.markOnboardingComplete()
            lines.append("isOnboardingComplete after markOnboardingComplete: \(model.isOnboardingComplete)")
        }

        lines.append("last action error: \(model.lastActionError ?? "none")")
        return lines.joined(separator: "\n") + "\n"
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
