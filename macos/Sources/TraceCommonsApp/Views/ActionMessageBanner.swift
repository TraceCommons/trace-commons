import SwiftUI

/// Displays one of the model's one-line action messages verbatim with a local
/// dismiss control. Dismissal does not retry the action or clear the daemon's
/// health state.
///
/// Both messages render through this one view. `lastActionError` -- what did
/// not happen -- got the control in PR #639; `lastActionNotice` -- what did --
/// is the same shape and gets the same control rather than a second answer to
/// the same question. The banner is deliberately neutral in wording so it can
/// carry either: it says "message", never "error".
///
/// Dismissal clears the published value rather than setting a suppression
/// flag: `AppModel.perform` re-assigns `lastActionError` on every later
/// failure, so a genuine recurrence re-renders and a restart starts clean.
/// The x can race a failure landing in the same frame -- that message is
/// dismissed unread, which is acceptable because every action error is a
/// fixed label reproducible by re-running the action. Refusals are not
/// reachable here: witness and health refusals render through their own
/// surfaces, so dismissal can never become the way out of one.
///
/// A notice carries slightly more than an error does -- the project-ignore
/// reconciliation names a count the contributor cannot recover by repeating
/// the action -- so dismissal there is a deliberate act on a sentence the
/// person has in front of them, which is the same bargain every banner
/// makes. It is strictly better than the alternative it replaces, which was
/// a sentence that stayed on screen for the rest of the session.
struct ActionMessageBanner: View {
    let text: String
    let onDismiss: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: TC.Space.m) {
            Text(text)
                .font(TC.Font_.body)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
            Button(action: onDismiss) {
                Image(systemName: "xmark")
                    .imageScale(.small)
                    .foregroundStyle(TC.inkSecondary)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Dismiss this message")
            .help("Puts this message away. It does not retry anything.")
        }
        .padding(.vertical, TC.Space.m)
        .padding(.horizontal, TC.Space.md)
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard()
        .accessibilityElement(children: .contain)
    }
}
