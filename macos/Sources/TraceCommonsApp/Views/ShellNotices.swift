import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The notices every window that shows the contributor's traces puts above
/// everything else: the attached-daemon limits, grant voids, arming
/// rewordings, a gate hold, the offer to move a legacy invite identity and
/// the notice after it moved. One view, so the
/// monitor and the first-run pane cannot drift apart on what they tell.
///
/// Every sentence, title and button label on these cards comes from the core
/// across the ABI (`attach_copy`, `consent_copy`, `AppModel`'s notices); this
/// file lays them out and writes none. A card whose words cannot be read is
/// not drawn: a card with no words is worse than none.
struct ShellNotices: View {
    @EnvironmentObject private var model: AppModel
    /// The stack's own height, measured, so the scroll view below is only
    /// as tall as the notices up to `maxHeight`.
    @State private var contentHeight: CGFloat = 0

    /// The most room the notices take above the tabs. There is one arming
    /// rewording per armed folder, so a contributor with dozens of folders
    /// got dozens of cards in an unscrolled stack: they pushed the tabs and
    /// the pane below out of the window (owner feedback, 2026-10-11).
    /// Past this height the notices scroll and the tabs stay put.
    static let maxHeight: CGFloat = 320

    var body: some View {
        ScrollView(.vertical) {
            notices
                .background {
                    GeometryReader { proxy in
                        Color.clear.preference(key: ShellNoticesHeightKey.self, value: proxy.size.height)
                    }
                }
        }
        .scrollBounceBehavior(.basedOnSize)
        .frame(height: min(contentHeight, Self.maxHeight))
        .onPreferenceChange(ShellNoticesHeightKey.self) { contentHeight = $0 }
    }

    private var notices: some View {
        VStack(spacing: GlassTokens.Space.s2) {
            // Two or more cards that each take a "Got it" get one press for
            // all of them (owner feedback, 2026-10-11). The word is the
            // core's; with no word, no button.
            if model.acknowledgeableNoticeCount > 1, let dismissAll = ActionNoticeWords.coreDismissAllWord {
                HStack {
                    Spacer()
                    Button(dismissAll) { model.acknowledgeAllNotices() }
                        .buttonStyle(GlassButtonStyle(.link))
                        .lineLimit(1)
                }
            }
            if model.isAttachedDaemon { AttachedDaemonNotice() }
            // Above the shell for the same reason: a void changes what the
            // contributor agreed to, and they are told wherever they are.
            GrantVoidNotices(
                voids: model.status.grantVoids,
                refused: model.grantVoidRearmRefused,
                onAcknowledge: { id in model.acknowledgeGrantVoid(id: id) },
                onRearm: { id, projectID in model.rearmGrantVoid(id: id, projectID: projectID) }
            )
            // The switch-on notices, above the shell for the same reason: a
            // folder's arming was reworded, or armed folders are on hold.
            ArmingRewordingNotices(
                rewordings: model.status.armingRewordings,
                refused: model.askFirstRefused,
                onAcknowledge: { id in model.acknowledgeArmingRewording(id: id) },
                onAskFirst: { projectID in model.askFirst(projectID: projectID) }
            )
            if let held = model.gateHeldNotice {
                GateHeldNoticeCard(
                    notice: held,
                    refused: model.askFirstRefused,
                    onAskFirst: { projectID in model.askFirst(projectID: projectID) }
                )
            }
            // A finished first run's last word (Automatic was refused, so
            // sharing is on Ask). Here because the first-run host is gone
            // by the time it can be read.
            if let notice = model.firstRunNotice {
                GlassNotice(tone: .ask, title: notice) {
                    Button(ActionNoticeWords.coreDismissWord ?? ActionNoticeWords.dismissWord) {
                        model.firstRunNotice = nil
                    }
                    .buttonStyle(GlassButtonStyle(.glass))
                }
            }
            // The contributor is told, wherever they are, that their
            // contributions now go under their NEAR AI account (the consent
            // spec requires it in every shell).
            LegacyMigrationNoticeCard(
                notice: model.legacyMigrationNotice,
                onAcknowledge: { model.acknowledgeLegacyInviteMigration() }
            )
            // The offer to move, on a device enrolled by an invite, until it
            // is moved or the card is dismissed. Settings keeps the offer.
            LegacyMigrationOfferCard(
                offer: model.showsLegacyMigrationOfferCard ? model.legacyMigrationOffer : nil,
                onDismiss: { model.dismissLegacyMigrationOffer() }
            )
        }
    }
}

private struct ShellNoticesHeightKey: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = max(value, nextValue())
    }
}

/// Said when this shell is driving a watcher that belongs to another
/// process.
///
/// Drawn rather than left silent because two of this window's controls stop
/// working and a contributor who is not told meets them as dead buttons:
/// the watcher cannot be stopped from here, and a trace's redacted body
/// cannot be opened -- the socket carries the summary only, so showing one
/// where a body was asked for would be a content promise the attached path
/// cannot keep.
struct AttachedDaemonNotice: View {
    var body: some View {
        // Both sentences come from `attach_copy` across the ABI. Drawing
        // nothing when it cannot answer is deliberate: the controls it warns
        // about still refuse with their own reasons.
        if let copy = TCAttach.copy() {
            GlassNotice(tone: .off, title: copy.attachedTitle) {
                Text(copy.attachedDetail)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

/// One bullet in a notice's list of reasons. The dot is decoration: the
/// reason's own words are the element.
private struct NoticeBullet<Content: View>: View {
    private let content: Content

    init(@ViewBuilder content: () -> Content) { self.content = content() }

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s2) {
            Image(systemName: "circle.fill")
                .imageScale(.small)
                .scaleEffect(0.4)
                .accessibilityHidden(true)
            content.fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// A heading line inside a notice, in the primary text colour.
private struct NoticeHeading: View {
    private let words: String

    init(_ words: String) { self.words = words }

    var body: some View {
        Text(words)
            .fontWeight(.semibold)
            .foregroundStyle(GlassColor.textPrimary)
            .fixedSize(horizontal: false, vertical: true)
    }
}

/// Every grant the daemon voided that no shell has shown yet (R6 of the
/// connect-and-forget design). The words come from `consent_copy` across the
/// ABI; this view only lays them out.
struct GrantVoidNotices: View {
    let voids: [GrantVoidWire]
    let refused: Set<UInt64>
    let onAcknowledge: (UInt64) -> Void
    let onRearm: (UInt64, String) -> Void

    var body: some View {
        if !voids.isEmpty {
            VStack(spacing: GlassTokens.Space.s2) {
                ForEach(voids, id: \.id) { void in
                    // The ABI words every element, including one it cannot
                    // place, so nil here is a caught panic or a payload this
                    // build cannot decode. Nothing is drawn then, as with
                    // `AttachedDaemonNotice`, and no sentence is written in
                    // this shell.
                    if let notice = TCConsentCopy.voidNoticeJSON(forVoid: void.json)
                        .flatMap(GrantVoidNotice.decode(fromJSON:))
                    {
                        GrantVoidNoticeCard(
                            notice: notice,
                            refused: refused.contains(void.id),
                            onAcknowledge: { onAcknowledge(void.id) },
                            onRearm: notice.rearmTarget(for: void).map { projectID in
                                { onRearm(void.id, projectID) }
                            }
                        )
                    }
                }
            }
        }
    }
}

/// The notice after a legacy invite identity moved to a NEAR AI account. The
/// words come from `consent_copy` across the ABI, worded once per notice by
/// `AppModel.legacyMigrationNotice`; this view only lays them out, and draws
/// nothing when there is no notice or it cannot be read.
struct LegacyMigrationNoticeCard: View {
    let notice: LegacyMigrationNotice?
    let onAcknowledge: () -> Void

    var body: some View {
        if let notice {
            GlassNotice(tone: .ask, title: notice.title) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                    Text(notice.body).fixedSize(horizontal: false, vertical: true)
                    Text(notice.folders).fixedSize(horizontal: false, vertical: true)
                    // Records that the notice was shown, and does nothing
                    // else.
                    Button(notice.acknowledge, action: onAcknowledge)
                        .buttonStyle(GlassButtonStyle(.glass))
                        .lineLimit(1)
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityLabel(Text(notice.title))
        }
    }
}

/// The offer to move a legacy invite identity to a NEAR AI account, as a
/// card above the tabs. The words come from `consent_copy` across the ABI
/// (`AppModel.legacyMigrationOffer`); this view only lays them out, and
/// draws nothing when there is no offer to show.
struct LegacyMigrationOfferCard: View {
    let offer: LegacyMigrationOffer?
    let onDismiss: () -> Void

    var body: some View {
        if let offer {
            GlassNotice(tone: .ask, title: offer.title) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                    Text(offer.body).fixedSize(horizontal: false, vertical: true)
                    Text(offer.later).fixedSize(horizontal: false, vertical: true)
                    LegacyMigrationMoveControls(offer: offer, placement: .notice, onDismiss: onDismiss)
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityLabel(Text(offer.title))
        }
    }
}

/// Move, the invite field once the daemon asks for it, and how the move
/// ended. Drawn by the card above the tabs and by Settings, so both say a
/// refusal the same way. Every word is the core's: the refusal is
/// `AppModel.legacyMigrationRefusal`, worded from the daemon's label.
struct LegacyMigrationMoveControls: View {
    @EnvironmentObject private var model: AppModel
    let offer: LegacyMigrationOffer
    /// The card or Settings: names the Move button apart from the other's.
    let placement: LegacyMigrationOffer.Placement
    /// The card's dismiss. Nil in Settings, where the offer stays for as
    /// long as the daemon makes it.
    var onDismiss: (() -> Void)? = nil
    @State private var invite = ""

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            if model.legacyMigrationInviteNeeded {
                GlassTextField(offer.invitePrompt, text: $invite)
                    .accessibilityLabel(offer.invitePrompt)
            }
            // The action first, the dismiss after it as a link (Ron,
            // 2026-10-09).
            HStack(spacing: GlassTokens.Space.s4) {
                if model.legacyMigrationAwaitingNearAILogin {
                    // The NEAR AI login step runs in the browser; Cancel stops
                    // waiting on it and moves nothing.
                    Text(offer.nearAILoginWaiting)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityAddTraits(.updatesFrequently)
                    Button(offer.nearAILoginCancel) { model.cancelLegacyMigrationNearAILogin() }
                        .buttonStyle(GlassButtonStyle(.link))
                        .lineLimit(1)
                } else if model.legacyMigrationBusy {
                    Text(offer.working)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityAddTraits(.updatesFrequently)
                } else if model.offersLegacyMigrationMove,
                    Self.canMove(inviteNeeded: model.legacyMigrationInviteNeeded, invite: invite)
                {
                    Button(offer.action) { Task { await model.migrateLegacyInvite(invite: invite) } }
                        .buttonStyle(GlassButtonStyle(.primary))
                        .lineLimit(1)
                        .accessibilityLabel(offer.actionName(for: placement))
                }
                // Puts the card away and moves nothing.
                if let onDismiss {
                    Button(offer.dismiss, action: onDismiss)
                        .buttonStyle(GlassButtonStyle(.link))
                        .lineLimit(1)
                }
            }
            // A refused or unstarted move, under the buttons it is about.
            if let refusal = model.legacyMigrationRefusal {
                GlassAlert(refusal)
            } else if model.legacyMigrationNearAILoginFailed {
                GlassAlert(offer.nearAILoginFailed)
            } else if model.legacyMigrationStartFailed {
                GlassAlert(offer.startFailed)
            }
        }
    }

    /// Move is hidden, not disabled, while the daemon has asked for the
    /// invite and none has been pasted: pressing it could only be refused
    /// the same way again.
    static func canMove(inviteNeeded: Bool, invite: String) -> Bool {
        !inviteNeeded || !invite.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}

struct GrantVoidNoticeCard: View {
    let notice: GrantVoidNotice
    let refused: Bool
    let onAcknowledge: () -> Void
    /// Present only when the Rust offered "Turn back on" and the element
    /// names a project: never on the grant's notice or an unplaced one.
    let onRearm: (() -> Void)?

    var body: some View {
        GlassNotice(tone: .ask, title: notice.title) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                Text(notice.body).fixedSize(horizontal: false, vertical: true)
                NoticeHeading(notice.reasonsHeading)
                ForEach(notice.reasons, id: \.self) { reason in
                    NoticeBullet { Text(reason) }
                }
                Text(notice.rearm).fixedSize(horizontal: false, vertical: true)
                // The action first, the acknowledgement after it as a link
                // (Ron, 2026-10-09).
                HStack(spacing: GlassTokens.Space.s4) {
                    // "Turn back on" sits beside the sentence saying that
                    // doing so agrees to the new settings. It is Settings'
                    // arming call.
                    if let onRearm, let action = notice.rearmAction {
                        Button(action, action: onRearm)
                            .buttonStyle(GlassButtonStyle(.primary))
                            .lineLimit(1)
                    }
                    // Acknowledging records that the notice was shown, and
                    // re-arms nothing.
                    Button(notice.acknowledge, action: onAcknowledge)
                        .buttonStyle(GlassButtonStyle(.link))
                        .lineLimit(1)
                }
                // A refused re-arm, under the buttons it is about.
                if refused, let failed = notice.rearmFailed {
                    GlassAlert(failed)
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(Text(notice.title))
    }
}

/// Every armed folder whose arming wording no longer claims a model scrubs
/// its sessions, not yet shown by any shell (K5). The words come from
/// `consent_copy` across the ABI; this view only lays them out. As with
/// `GrantVoidNotices`, nothing is drawn for an element the ABI cannot word.
struct ArmingRewordingNotices: View {
    let rewordings: [ArmingRewordingWire]
    let refused: Set<String>
    let onAcknowledge: (UInt64) -> Void
    let onAskFirst: (String) -> Void

    var body: some View {
        if !rewordings.isEmpty {
            VStack(spacing: GlassTokens.Space.s2) {
                ForEach(rewordings, id: \.id) { rewording in
                    if let notice = TCConsentCopy.armingRewordedNoticeJSON(forRewording: rewording.json)
                        .flatMap(ArmingRewordedNotice.decode(fromJSON:))
                    {
                        let target = notice.askFirstTarget(for: rewording)
                        ArmingRewordedNoticeCard(
                            notice: notice,
                            refused: target.map(refused.contains) ?? false,
                            onAcknowledge: { onAcknowledge(rewording.id) },
                            onAskFirst: target.map { projectID in { onAskFirst(projectID) } }
                        )
                    }
                }
            }
        }
    }
}

struct ArmingRewordedNoticeCard: View {
    let notice: ArmingRewordedNotice
    let refused: Bool
    let onAcknowledge: () -> Void
    /// Present only when the Rust offered "Ask" and the element
    /// names a project.
    let onAskFirst: (() -> Void)?

    var body: some View {
        GlassNotice(tone: .ask, title: notice.title) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                Text(notice.body).fixedSize(horizontal: false, vertical: true)
                NoticeHeading(notice.nowHeading)
                ForEach([notice.scope, notice.limit, notice.noReview], id: \.self) { line in
                    Text(line).fixedSize(horizontal: false, vertical: true)
                }
                // The action first, the acknowledgement after it as a link
                // (Ron, 2026-10-09).
                HStack(spacing: GlassTokens.Space.s4) {
                    if let onAskFirst, let action = notice.askFirstAction {
                        Button(action, action: onAskFirst)
                            .buttonStyle(GlassButtonStyle(.primary))
                            .lineLimit(1)
                    }
                    // Acknowledging records that the notice was shown, and
                    // changes nothing about the folder.
                    Button(notice.acknowledge, action: onAcknowledge)
                        .buttonStyle(GlassButtonStyle(.link))
                        .lineLimit(1)
                }
                // A refused Ask first, under the buttons it is about.
                if refused, let failed = notice.askFirstFailed {
                    GlassAlert(failed)
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(Text(notice.title))
    }
}

/// Armed folders the automatic-contribution gate is holding, in the Rust's
/// words. No dismiss button: it goes when the hold does.
struct GateHeldNoticeCard: View {
    let notice: GateHeldNotice
    let refused: Set<String>
    let onAskFirst: (String) -> Void

    var body: some View {
        GlassNotice(tone: .ask, title: notice.title) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                Text(notice.body).fixedSize(horizontal: false, vertical: true)
                ForEach(notice.reasons, id: \.self) { reason in
                    NoticeBullet { Text(reason) }
                }
                Text(notice.release).fixedSize(horizontal: false, vertical: true)
                if !notice.projects.isEmpty {
                    Text(notice.askFirst).fixedSize(horizontal: false, vertical: true)
                    ForEach(notice.projects, id: \.line) { project in
                        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                            Text(project.line)
                                .foregroundStyle(GlassColor.textPrimary)
                                .fixedSize(horizontal: false, vertical: true)
                            if let projectID = project.projectId, let action = project.askFirstAction {
                                Button(action) { onAskFirst(projectID) }
                                    .buttonStyle(GlassButtonStyle(.glass))
                                    .lineLimit(1)
                            }
                            if let projectID = project.projectId, refused.contains(projectID),
                                let failed = project.askFirstFailed
                            {
                                GlassAlert(failed)
                            }
                        }
                    }
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(Text(notice.title))
    }
}
