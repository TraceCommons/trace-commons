import AppKit
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// What discovery has given the Tools step so far. The file keeps the name
/// of Ron's #1030 Folders screen, whose rules Tools now follows.
enum DiscoveredRows: Equatable {
    /// Discovery has not answered yet.
    case loading
    /// Discovery answered with no row the shell could read: nil JSON (a
    /// caught panic), an undecodable array, or an empty one. Claude Code and
    /// Codex are always emitted, so an empty list is a failure too.
    case failed
    case found([SourceCandidate])

    var rows: [SourceCandidate]? {
        if case .found(let rows) = self { return rows }
        return nil
    }

    /// The core's line for a failed discovery, shown with a retry.
    func failureLine(_ copy: FirstRunCopy.Folders) -> String? {
        self == .failed ? copy.discoveryFailed : nil
    }
}

/// The decisions Ron's Folders screen and Tools shared, which Tools keeps,
/// apart from the view so they can be tested.
enum FoldersScreenLayout {
    /// Discovery's rows, read without a daemon. A refresh that fails keeps
    /// the rows already shown; answers live in the run's state, keyed by
    /// kind, so a refresh that succeeds keeps them too.
    static func discovered(_ json: String?, keeping previous: DiscoveredRows) -> DiscoveredRows {
        if let json, let rows = try? SourceCandidate.decodeList(from: json), !rows.isEmpty {
            return .found(rows)
        }
        if case .found = previous { return previous }
        return .failed
    }

    /// The rows take no answer while a commit runs: `.start` sends no roots,
    /// so a row changed mid-commit would show a declaration the daemon never
    /// received.
    static func rowsEnabled(isCommitting: Bool) -> Bool {
        !isCommitting
    }

    /// The core's sentence for a failure Tools or Join can show. A failed
    /// start reads `watcher_start_failed` and the step stays; a changed
    /// declaration the running daemon refused reads `settings_failed`. A failed enroll reads
    /// `enroll_refused`: lookup has already accepted the invite, so Join's
    /// `invite_error` ("not an invite link") would be false, and the daemon
    /// never says why enroll refused. An invite the issuer could not be asked
    /// about reads `lookup_unavailable`, which says nothing of the invite, and
    /// a near.ai sign-in that did not finish reads the core's sign-in line
    /// (`consent_copy::INFERENCE_SIGN_IN_FAILED`). A dead invite has already
    /// returned the person to Join, which shows its own line.
    static func notice(
        for failure: FirstRunFailure?, copy: FirstRunCopy, onboarding: TCOnboardingCopy?
    ) -> String? {
        switch failure {
        case .startFailed?: return onboarding?.watcherStartFailed
        case .settingsFailed?: return copy.folders.settingsFailed
        case .enrollFailed?: return copy.folders.enrollRefused
        case .lookupUnavailable?: return copy.folders.lookupUnavailable
        case .signInFailed?: return copy.folders.signInFailed
        // The core's line for the daemon's label, as the near.ai join view
        // words it; a label it has no line for reads the enroll refusal.
        case .nearAIEnrollFailed(let label)?: return TCNearAiEnroll.line(label: label) ?? copy.folders.enrollRefused
        default: return nil
        }
    }

    /// Cancel beside the spinning sign-in, only while the near.ai browser
    /// sign-in runs (`FirstRunRunner.signInWaiting`), in the core's
    /// first-run Cancel.
    static func signInCancel(
        waiting: Bool, copy: FirstRunCopy, action: @escaping () -> Void
    ) -> FirstRunFooter.Cancel? {
        waiting ? FirstRunFooter.Cancel(title: copy.frame.cancel, action: action) : nil
    }
}
