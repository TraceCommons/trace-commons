import SwiftUI
import TCDesign

/// The go-public acknowledgement gate: the primary does nothing until there
/// is a handle to consent to, a consent to it, and no call in flight.
enum GoPublicGate {
    static func canGoPublic(acknowledged: Bool, handle: String, busy: Bool) -> Bool {
        acknowledged
            && !handle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !busy
    }
}

/// A bio box and its byte counter. Bytes, because the limit is stated in
/// bytes; the counter is counted off the value, never typed.
private struct GlassBioEditor: View {
    let label: String
    @Binding var text: String

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            Text(label)
                .glassType(GlassTokens.TypeScale.eyebrow)
                .foregroundStyle(GlassColor.textTertiary)
            GlassWell {
                TextEditor(text: $text)
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textPrimary)
                    .scrollContentBackground(.hidden)
                    .frame(minHeight: 56)
                    .padding(GlassTokens.Space.s2)
                    .accessibilityLabel(label)
            }
            Text("\(text.utf8.count)/280")
                .glassType(GlassTokens.TypeScale.mono)
                .foregroundStyle(GlassColor.textSecondary)
                .frame(maxWidth: .infinity, alignment: .trailing)
        }
    }
}

struct PublicProfileSection: View {
    @EnvironmentObject private var model: AppModel
    @State private var showingGoPublic = false
    /// The panel's two editable fields. Seeded from the daemon's answer --
    /// see `seedProfileDraft` -- rather than bound straight to it, so a
    /// background refresh cannot rewrite what is being typed.
    @State private var handleDraft = ""
    @State private var bioDraft = ""

    private static let rosterDate: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateStyle = .long
        formatter.timeStyle = .none
        return formatter
    }()

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            if let profile = model.publicProfile, let handle = profile.handle {
                profilePanel(profile, handle: handle)
            } else {
                optInCard
            }
            if let sentence = profileOutcomeSentence {
                GlassNotice(tone: outcomeTone) { Text(sentence) }
            }
            Text(PublicProfileCopy.footnote)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            profileCopyDefects
        }
        // Seeded from the daemon's answer whenever it changes, so the fields
        // show what is actually published -- including the trimmed display
        // form the server stored. Keyed on the published values rather than
        // on every render, so a refresh cannot overwrite an edit in progress.
        .onAppear { seedProfileDraft() }
        .onChange(of: publishedSignature) { _, _ in seedProfileDraft() }
        .sheet(isPresented: $showingGoPublic) {
            // Handed the model explicitly: the sheet makes a daemon call, and
            // an environment object it did not get would be a crash on the
            // one button that matters.
            GoPublicSheet(onDismiss: { showingGoPublic = false })
                .environmentObject(model)
        }
    }

    /// Off the roster: a button that opens the consent sheet rather than
    /// doing anything itself. With no daemon answer the button is disabled.
    private var optInCard: some View {
        GlassEyebrowCard(PublicProfileCopy.heading) {
            HStack(alignment: .center, spacing: GlassTokens.Space.s3) {
                Text(PublicProfileCopy.listHandlePublicly)
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textPrimary)
                Spacer(minLength: 0)
                Button(PublicProfileCopy.goPublicConfirm) { showingGoPublic = true }
                    .buttonStyle(GlassButtonStyle(.glass))
                    .disabled(!model.status.loggedIn)
            }
        }
    }

    /// On the roster, editable.
    private func profilePanel(_ profile: DaemonClient.PublicProfile, handle: String) -> some View {
        GlassEyebrowCard(PublicProfileCopy.heading) {
            if let since = profile.publicSince {
                GlassTag(PublicProfileCopy.onRosterSince(Self.rosterDate.string(from: since)), tone: .accent)
            }
        } content: {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                GlassTextField(PublicProfileCopy.handleLabel, text: $handleDraft)
                GlassBioEditor(label: PublicProfileCopy.bioLabel, text: $bioDraft)
                HStack(spacing: GlassTokens.Space.s2) {
                    // Save re-publishes the whole profile, because that is
                    // what the PUT does: the handle and the bio as they
                    // stand, both of them, every time.
                    Button(PublicProfileCopy.saveProfile) {
                        model.claimHandle(handleDraft, bio: bioDraft)
                    }
                    .buttonStyle(GlassButtonStyle(.primary))
                    .disabled(model.profileBusy || handleDraft.trimmingCharacters(
                        in: .whitespacesAndNewlines
                    ).isEmpty)
                    Button(PublicProfileCopy.leaveRoster) { model.leaveRoster() }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .disabled(model.profileBusy)
                }
            }
        }
        // The handle is what is published; VoiceOver hears it rather than
        // unlabelled boxes, and the fields stay reachable inside it.
        .accessibilityElement(children: .contain)
        .accessibilityLabel("\(PublicProfileCopy.heading): \(handle)")
    }

    /// The public-profile copy's own assertions, rendered where a
    /// contributor and a developer both see them. Empty in every healthy
    /// build.
    @ViewBuilder
    private var profileCopyDefects: some View {
        let problems = PublicProfileCopyCheck.failures()
        if !problems.isEmpty {
            GlassNotice(tone: .outside, title: SettingsLegacyWords.doNotTrustProfileWording) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                    ForEach(problems, id: \.self) { problem in
                        Text(problem)
                    }
                }
            }
        }
    }

    /// What the last claim or withdrawal did, in words.
    ///
    /// `published(cached: false)` is a **success**: the server has taken the
    /// handle, and only this device's copy of it is missing. It gets the
    /// sentence that says so rather than a refusal sentence.
    private var profileOutcomeSentence: String? {
        switch model.profileOutcome {
        case .none: return nil
        case .published(let cached):
            return cached ? PublicProfileCopy.published : PublicProfileCopy.publishedNotCached
        case .left(let cached):
            return cached ? PublicProfileCopy.leftRoster : PublicProfileCopy.leftRosterNotCached
        case .refused(let label):
            return PublicProfileCopy.failureSentence(label)
        case .leaveRefused(let label):
            return PublicProfileCopy.leaveFailureSentence(label)
        }
    }

    private var outcomeTone: GlassStatus {
        switch model.profileOutcome {
        case .refused, .leaveRefused: return .outside
        default: return .on
        }
    }

    /// The published values, as one string, so the drafts are re-seeded when
    /// and only when the daemon's answer actually changes.
    private var publishedSignature: String {
        "\(model.publicProfile?.handle ?? "")\u{1}\(model.publicProfile?.bio ?? "")"
    }

    private func seedProfileDraft() {
        handleDraft = model.publicProfile?.handle ?? ""
        bioDraft = model.publicProfile?.bio ?? ""
    }
}

/// Going public is a deliberate consent dialog, not a toggle flip: what gets
/// published and what never does sit side by side, nothing is pre-checked,
/// and "Go public" stays disabled until the acknowledgement is checked.
struct GoPublicSheet: View {
    var onDismiss: () -> Void

    @EnvironmentObject private var model: AppModel
    @State private var acknowledged = false
    @State private var handle = ""
    @State private var bio = ""

    var body: some View {
        GlassSheet(title: PublicProfileCopy.goPublicHeadline) {
            HStack(alignment: .top, spacing: GlassTokens.Space.cardGap) {
                column(PublicProfileCopy.publishedHeading, SettingsLegacyWords.publishedLines)
                column(PublicProfileCopy.neverHeading, SettingsLegacyWords.neverLines)
            }
            // The handle is inside the consent dialog rather than behind it:
            // the thing consented to is this exact string becoming public.
            GlassTextField(PublicProfileCopy.goPublicHandleLabel, text: $handle)
            GlassBioEditor(label: PublicProfileCopy.goPublicBioLabel, text: $bio)
            Toggle(PublicProfileCopy.goPublicAcknowledgement, isOn: $acknowledged)
                .toggleStyle(GlassCheckboxStyle())
            // A refusal stays in the dialog, next to the field it is about.
            if case .refused(let label) = model.profileOutcome {
                GlassNotice(tone: .outside) { Text(PublicProfileCopy.failureSentence(label)) }
            }
            HStack {
                Spacer(minLength: 0)
                Button(PublicProfileCopy.notNow, action: onDismiss).buttonStyle(GlassButtonStyle(.glass))
                Button(PublicProfileCopy.goPublicConfirm) { model.claimHandle(handle, bio: bio) }
                    .buttonStyle(GlassButtonStyle(.primary))
                    .disabled(!GoPublicGate.canGoPublic(acknowledged: acknowledged, handle: handle, busy: model.profileBusy))
            }
            Text(PublicProfileCopy.goPublicFootnote)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(width: 560)
        // Any outcome that is not a refusal is a claim the server accepted,
        // including one this device failed to cache: the handle is on the
        // roster either way, so the sheet's work is done.
        .onChange(of: outcomeIsSettled) { _, settled in
            if settled { onDismiss() }
        }
        // A stale refusal from an earlier attempt must not greet the next
        // opening of this sheet.
        .onAppear { model.clearProfileOutcome() }
    }

    private var outcomeIsSettled: Bool {
        switch model.profileOutcome {
        case .published, .left: return true
        case .none, .refused, .leaveRefused: return false
        }
    }

    private func column(_ title: String, _ lines: [String]) -> some View {
        GlassWell {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                Text(title)
                    .glassType(GlassTokens.TypeScale.eyebrow)
                    .foregroundStyle(GlassColor.textTertiary)
                ForEach(lines, id: \.self) { line in
                    Text(line)
                        .glassType(GlassTokens.TypeScale.body)
                        .foregroundStyle(GlassColor.textPrimary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(GlassTokens.Space.s3)
        }
    }
}
