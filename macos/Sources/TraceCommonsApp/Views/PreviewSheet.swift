import AppKit
import SwiftUI
import TCBridge
import TCShellCore

/// "Look inside": the one surface in the product that deliberately shows
/// trace content, because consent to send something you cannot see is not
/// consent.
///
/// Four tabs, in the spec's order. **Search is first and focused** on
/// purpose: "does this mention my client's name?" is a question a
/// contributor can answer in five seconds. Judging redaction quality by eye
/// is not, and this interface never asks them to.
///
/// **One sheet, one session, one decision.** Both decisions close it and put
/// the contributor back on the queue. It used to load the next waiting
/// session into itself with `Contribute` under the same pixels, which made a
/// second click -- or a second Return, since the button was the default
/// action -- send a transcript nobody had looked at, with the recovery bar
/// stranded behind the sheet where it could not be seen.
///
/// `Contribute` waits only on a loaded preview (see `TCShellCore.ReadGate`)
/// and is bound to no keyboard shortcut at all.
///
/// The layout follows `design-import/DESIGN-SPEC.md` §5.2 (`1b` preview
/// sheet) and §5.10 (`4a` transcript renderer), which are the same shell:
/// header bar, tab strip, body, footer bar, with the header's second field
/// and the body swapping when the transcript tab is active.
struct PreviewSheet: View {
    /// Content already loaded elsewhere, so the sheet can be rendered
    /// without running its `task`. Used only by the screenshot hook, which
    /// has to rasterize the real view (`ImageRenderer` never runs `task` or
    /// `onAppear`) rather than photograph a window.
    struct Preloaded {
        let summary: PreviewSummary
        let transcript: String
        let needle: String
        let offsets: [Int]
    }

    let entry: QueueEntry
    let preloaded: Preloaded?

    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var preview: TCPreview?
    @State private var summary: PreviewSummary?
    @State private var transcriptText: String
    /// The transcript cut into chunks, built once when the body arrives.
    ///
    /// One document serves both tabs that need to walk the body: the
    /// transcript tab pages through its chunks, and the search tab cuts its
    /// context snippets out of its bytes. The search tab used to build
    /// `Array(transcript.utf8)` inside a computed property -- a full copy of
    /// the body per keystroke, 17.5 MB at a time on a real session.
    @State private var document: TranscriptDocument?
    @State private var failure: String?
    @State private var loading: Bool

    /// The contributor's answer to `VerdictCopy.question`, or `nil` for the
    /// answer they did not give.
    ///
    /// `nil` is the starting state and a perfectly good ending one: it never
    /// gates `Contribute`, and it is sent as an ABSENT `outcome`, not an
    /// empty one. One sheet is one session's decision, and the sheet is
    /// rebuilt per entry, so no verdict can carry into the next.
    @State private var verdict: ContributorVerdict?

    /// What the contributor wrote in the correction box.
    ///
    /// Shown only under `.partly` and `.failed` -- you cannot correct a run
    /// you have just called successful, and that gate is a guard as much as
    /// it is semantics: it halves the surface for correction-shaped credit
    /// farming and puts the field only where a correction means something.
    ///
    /// Optional throughout, and it never gates `Contribute`. Emptied when
    /// the answer moves off those two, because text left in a hidden box
    /// would ride along on an approval without ever being on screen again.
    @State private var correction: String = ""

    /// Set when the daemon refused this submission because the correction
    /// contains something credential-shaped. The sheet stays open with the
    /// text still in the box: the next thing the contributor has to do is
    /// edit it.
    @State private var correctionRefused = false

    // MARK: - The read gate
    //
    // There is no longer a read gate. `Contribute` used to wait on the
    // transcript tab having been on screen AND an acknowledgement checkbox
    // ticked by hand; both are gone, and `TCShellCore.ReadGate` records why
    // and holds the sentence that took their place. What survives here is
    // the one condition that was never friction: a preview has to have
    // loaded, because that is what an approval binds to.

    /// Search first, always: it is the question a contributor can actually
    /// answer in five seconds.
    @State private var tab: Tab = .search

    enum Tab: String, CaseIterable, Identifiable {
        case search, whatsInIt, transcript, permissions
        var id: String { rawValue }
        var title: String {
            switch self {
            case .search: return "Search"
            case .whatsInIt: return "What's in it"
            case .transcript: return "Exactly what would be sent"
            case .permissions: return "Permissions"
            }
        }

        var symbol: String {
            switch self {
            case .search: return "magnifyingglass"
            case .whatsInIt: return "list.bullet.rectangle"
            case .transcript: return "doc.plaintext"
            case .permissions: return "checklist"
            }
        }
    }

    init(entry: QueueEntry, preloaded: Preloaded? = nil) {
        self.entry = entry
        self.preloaded = preloaded
        _summary = State(initialValue: preloaded?.summary)
        _transcriptText = State(initialValue: preloaded?.transcript ?? "")
        _document = State(initialValue: preloaded.map { TranscriptDocument($0.transcript) })
        _loading = State(initialValue: preloaded == nil)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            SheetHairline()
            content
            SheetHairline()
            footer
        }
        .frame(width: SheetMetric.width, height: SheetMetric.height)
        .tcScreen()
        .task(id: entry.entryID) {
            guard preloaded == nil else { return }
            await load()
        }
        .onDisappear { closePreview() }
        // The credential refusal, as its own alert rather than a line in
        // the submit toast: it is the one submit failure the contributor
        // caused and the only one they can fix, and it asks them to do two
        // things -- edit the text, and rotate what they typed. Neither
        // string is derived from the response, so no correction text and no
        // detected value can reach the screen a second time.
        .alert(
            CorrectionCopy.credentialHeadline,
            isPresented: $correctionRefused
        ) {
            Button("Close", role: .cancel) {}
        } message: {
            Text(CorrectionCopy.credentialBody)
        }
    }

    // MARK: - Chrome

    /// The same identity line and the same labelled figures as a queue card,
    /// in the same order. Recognising the card you just clicked is one of
    /// the quieter things that makes a preview trustworthy.
    ///
    /// The second field is not fixed: §5.10 replaces the "nothing sent yet"
    /// lock with what scrubbing actually found while the transcript is on
    /// screen, because that is the number a person reads the body against.
    private var header: some View {
        VStack(alignment: .leading, spacing: TC.Space.sm) {
            HStack(alignment: .firstTextBaseline, spacing: TC.Space.s) {
                Text(entry.projectLabel)
                    .font(TC.Font_.cardTitle)
                    .foregroundStyle(TC.inkPrimary)
                Text(entry.agentName)
                    .font(TC.Font_.caption)
                    .foregroundStyle(TC.inkSecondary)
                Spacer(minLength: TC.Space.m)
                Text(Format.when(entry.discoveredAt))
                    .font(TC.Font_.caption)
                    .foregroundStyle(TC.inkTertiary)
            }
            HStack(alignment: .firstTextBaseline, spacing: TC.Space.xxl) {
                if let summary {
                    VStack(alignment: .leading, spacing: TC.Space.micro) {
                        TCFieldLabel("Would send")
                        HStack(alignment: .firstTextBaseline, spacing: TC.Space.xs) {
                            Text(Format.bytes(summary.wouldSendBytes))
                                .font(TC.Font_.ledger)
                                .monospacedDigit()
                                .foregroundStyle(TC.inkPrimary)
                            if tab == .transcript {
                                Text("(the session file on disk is \(Format.bytes(summary.rawSessionBytes)))")
                                    .font(TC.Font_.caption)
                                    .foregroundStyle(TC.inkSecondary)
                            }
                        }
                    }
                    .accessibilityElement(children: .combine)
                }
                if tab == .transcript, let summary {
                    VStack(alignment: .leading, spacing: TC.Space.micro) {
                        TCFieldLabel("Scrubbing found")
                        Text(Self.scrubbingFound(summary))
                            .font(TC.Font_.ledger)
                            .foregroundStyle(
                                summary.redactions.isEmpty
                                    ? TC.Tone.attention.textColor
                                    : TC.inkPrimary
                            )
                    }
                    .accessibilityElement(children: .combine)
                } else {
                    VStack(alignment: .leading, spacing: TC.Space.micro) {
                        TCFieldLabel("Status")
                        TCTag(text: "nothing sent yet", tone: .clear, symbol: "lock")
                    }
                    .accessibilityElement(children: .combine)
                }
                Spacer(minLength: 0)
            }
            Text("Nothing has been sent. This is what would be.")
                .font(TC.Font_.caption)
                .foregroundStyle(TC.inkSecondary)
        }
        .padding(.horizontal, TC.Space.lg)
        .padding(.vertical, TC.Space.md)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(TC.surface)
    }

    /// "12 secrets · 4 file paths · 2 email addresses" -- category labels and
    /// counts, in the daemon's own words, largest first. The contract
    /// guarantees this map never carries matched text.
    private static func scrubbingFound(_ summary: PreviewSummary) -> String {
        // Removals only. `redactions` also carries `residual_secret_at:*`,
        // which counts a secret that was DETECTED AND LEFT IN, and this line
        // sits under a heading that says the opposite. See
        // `RedactionLabels`; the survivor is stated separately, in the
        // attention tone, rather than dropped.
        let removals = RedactionLabels.removals(summary.redactions)
        guard !removals.isEmpty else { return "nothing matched" }
        return removals
            .sorted { $0.value == $1.value ? $0.key < $1.key : $0.value > $1.value }
            .map { "\($0.value) \($0.key.replacingOccurrences(of: "_", with: " "))" }
            .joined(separator: " · ")
    }

    @ViewBuilder
    private var content: some View {
        if loading {
            CenteredNotice(
                title: "Scrubbing it locally…",
                detail: "Reading the session and running the redaction pass."
            )
        } else if let failure {
            CenteredNotice(
                title: "This one can't be shown.",
                detail: """
                \(failure). Nothing has been sent, and nothing will be until it can \
                be shown to you.
                """
            )
        } else if let summary {
            // A segmented control rather than a TabView: inside a sheet this
            // is the standard macOS treatment, and Search has to be able to
            // start selected and focused.
            //
            // It is built from Buttons rather than `Picker(.segmented)`
            // because each segment needs to carry a glyph AND a count -- the
            // number of things scrubbing removed sits on "What's in it", so
            // a person can see there is something to look at before they
            // click the tab. A stock segmented picker takes labels only.
            VStack(alignment: .leading, spacing: TC.Space.m) {
                tabBar(summary)

                switch tab {
                case .search:
                    SearchTab(
                        document: document,
                        preview: preview,
                        initialNeedle: preloaded?.needle ?? "",
                        initialOffsets: preloaded?.offsets
                    )
                case .whatsInIt:
                    WhatsInItTab(entry: entry, summary: summary)
                case .transcript:
                    // Fail closed rather than cutting a fresh document on
                    // every layout pass. Unreachable in practice -- the
                    // document is built in the same step that decodes the
                    // summary, and this branch only renders once the
                    // summary exists.
                    if let document {
                        TranscriptTab(document: document)
                    } else {
                        CenteredNotice(
                            title: "The transcript isn't ready.",
                            detail: """
                            Nothing has been sent, and nothing will be until it can be \
                            shown to you.
                            """
                        )
                    }
                case .permissions:
                    PermissionsTab(summary: summary, options: model.consentScopes)
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, TC.Space.lg)
            .padding(.vertical, TC.Space.md)
        }
    }

    /// The four tabs, in the spec's order, each a plain button. The one that
    /// has something to report says so on its face.
    private func tabBar(_ summary: PreviewSummary) -> some View {
        HStack(spacing: TC.Space.xxs) {
            ForEach(Tab.allCases) { item in
                Button {
                    tab = item
                } label: {
                    HStack(spacing: TC.Space.xxs) {
                        Image(systemName: item.symbol)
                            .imageScale(.small)
                        Text(item.title)
                            .font(TC.Font_.caption.weight(tab == item ? .bold : .regular))
                        if let note = badge(for: item, summary: summary) {
                            Text(note)
                                .font(TC.Font_.monoBadge)
                        }
                    }
                    .foregroundStyle(tab == item ? TC.inkPrimary : TC.inkSecondary)
                    .padding(.horizontal, TC.Space.m)
                    .padding(.vertical, TC.Space.control)
                    .background {
                        RoundedRectangle(cornerRadius: TC.Radius.control)
                            .fill(tab == item ? TC.surface : Color.clear)
                    }
                    .overlay {
                        RoundedRectangle(cornerRadius: TC.Radius.control)
                            .strokeBorder(
                                tab == item
                                    ? TC.green.opacity(TC.Border.activeTabAlpha)
                                    : Color.clear,
                                lineWidth: TC.Border.hairline
                            )
                    }
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(tab == item ? [.isSelected, .isButton] : .isButton)
            }
            Spacer(minLength: 0)
        }
        .padding(TC.Space.xxs)
        .background(TC.surfaceInset, in: RoundedRectangle(cornerRadius: TC.Radius.card))
    }

    private func badge(for item: Tab, summary: PreviewSummary) -> String? {
        switch item {
        case .whatsInIt:
            let removed = summary.redactions.values.reduce(0, +)
            return removed == 0 ? nil : "\(removed)"
        case .permissions:
            return "\(summary.consentScopes.count)"
        default:
            return nil
        }
    }

    /// The one irreversible click in the product, and the one place the
    /// scrubbing caveat is repeated verbatim on purpose -- see
    /// `ScrubbingCaveat`.
    private var footer: some View {
        VStack(alignment: .leading, spacing: TC.Space.sm) {
            // §5.10 drops this line from the transcript tab's footer. It is
            // kept on every tab here: it is the sentence everything else on
            // this sheet is qualified by, and the tab a person is standing
            // on when they reach for Contribute is not a reason to stop
            // saying it.
            ScrubbingCaveatAtCommit()
            gateStatement
            verdictQuestion
            if correctionIsOffered {
                correctionField
            }
            HStack(spacing: TC.Space.s) {
                // Outlined like "Close", never filled: it must not read as a
                // second way to approve.
                Button("Not this one") {
                    model.dismiss(entry)
                    dismiss()
                }
                .buttonStyle(SheetSecondaryButtonStyle())
                Spacer(minLength: TC.Space.m)
                Button("Close") { dismiss() }
                    .buttonStyle(SheetSecondaryButtonStyle())
                // The ONLY approve control in the product. It is behind the
                // preview by design -- it cannot arm until one has loaded --
                // and it has NO keyboard shortcut: this used to be
                // `.defaultAction`, which put an irreversible send one
                // Return away from a hand resting on the keyboard.
                Button("Contribute") {
                    contribute()
                }
                .tcPrimaryAction()
                .disabled(!canContribute)
                .help(gateHelp)
            }
        }
        .padding(.horizontal, TC.Space.lg)
        .padding(.vertical, TC.Space.md)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(TC.surface)
    }

    // MARK: - What arms Contribute

    private var canContribute: Bool {
        ReadGate.canContribute(hasPinnedPreview: summary != nil)
    }

    private var gateHelp: String {
        ReadGate.help(hasPinnedPreview: summary != nil)
    }

    /// The sentence the acknowledgement checkbox used to carry, printed
    /// where the tick used to be asked for.
    ///
    /// Plain text and no control: it is a statement the app makes, not a
    /// question it asks. The words are `TCShellCore.ReadGate.statement`
    /// rather than a literal here, because the Linux and Windows sheets
    /// print the same sentence and a copy in a view is a copy that drifts.
    /// The outcome question, its three answers, and the disclosure under
    /// them.
    ///
    /// Nothing here gates anything. There is no default selection, no
    /// fourth "didn't answer" option, and `Contribute` is armed by
    /// `ReadGate` alone -- a contributor who ignores this entire block
    /// contributes exactly as before, and the approval simply omits
    /// `outcome`. Tapping the selected answer again clears it, which is the
    /// only way back to unanswered once an answer is given.
    ///
    /// The caption is `VerdictCopy.caption` verbatim: it is where the sheet
    /// discloses that these fields sit outside its "exactly what would be
    /// sent" guarantee, which is the one thing on this sheet the preview
    /// cannot show.
    /// Whether the correction control is on screen: only under `Partly`
    /// and `Failed`.
    private var correctionIsOffered: Bool {
        verdict == .partly || verdict == .failed
    }

    /// What would actually be sent, or `nil` for a box that is hidden or
    /// holds nothing but whitespace.
    ///
    /// The visibility check is deliberate rather than redundant. The box is
    /// emptied when it is hidden, so this would answer `nil` anyway; the
    /// check states the rule -- a hidden control contributes nothing -- so
    /// it survives a future change that stops emptying on hide.
    private var correctionToSend: String? {
        guard correctionIsOffered else { return nil }
        return CorrectionCopy.toSend(correction)
    }

    /// Approve, and decide when the sheet goes away.
    ///
    /// With no correction, this is exactly what it was before the box
    /// existed: fire and dismiss. Back to the queue, never on to the next
    /// session -- the sheet used to load the next entry with the button
    /// under the same pixels, so a second keystroke or a second click sent
    /// a transcript nobody had looked at, with the recovery bar stranded
    /// behind the sheet. One sheet, one session, one decision.
    ///
    /// With a correction, the dismiss waits for the answer, because one of
    /// the answers is "that correction contains a credential" and the
    /// contributor needs the text still in front of them to act on it.
    private func contribute() {
        guard let text = correctionToSend else {
            model.approve(entry, verdict: verdict)
            dismiss()
            return
        }
        model.approve(entry, verdict: verdict, correction: text) { refused in
            if refused {
                correctionRefused = true
            } else {
                dismiss()
            }
        }
    }

    /// The correction box and the disclosure under it.
    ///
    /// `CorrectionCopy.caption` is printed verbatim and in full. Until the
    /// published policy page carves a correction out of its "redacted
    /// locally, re-applied on the server" promise, that sentence is the
    /// only place a contributor is told their own words are stored as they
    /// typed them. It is not shortened for layout.
    private var correctionField: some View {
        VStack(alignment: .leading, spacing: TC.Space.xxs) {
            Text(CorrectionCopy.question)
                .font(TC.Font_.captionSmall)
                .foregroundStyle(TC.inkSecondary)
            TextEditor(text: $correction)
                .font(TC.Font_.caption)
                .frame(minHeight: 64, maxHeight: 140)
                .scrollContentBackground(.hidden)
                .padding(TC.Space.xxs)
                .background(TC.surfaceInset, in: RoundedRectangle(cornerRadius: TC.Radius.card))
                .accessibilityLabel(CorrectionCopy.question)
                .accessibilityHint(CorrectionCopy.placeholder)
                // Capped where the person can see it, so an over-long
                // correction is shortened at the keyboard rather than
                // refused as `correction-too-long` after the click.
                .onChange(of: correction) { _, latest in
                    if latest.count > CorrectionCopy.maxCharacters {
                        correction = String(latest.prefix(CorrectionCopy.maxCharacters))
                    }
                }
            Text(CorrectionCopy.caption)
                .font(TC.Font_.captionSmall)
                .foregroundStyle(TC.inkTertiary)
                .fixedSize(horizontal: false, vertical: true)
                .multilineTextAlignment(.leading)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var verdictQuestion: some View {
        VStack(alignment: .leading, spacing: TC.Space.xxs) {
            Text(VerdictCopy.question)
                .font(TC.Font_.captionSmall)
                .foregroundStyle(TC.inkSecondary)
            HStack(spacing: TC.Space.xxs) {
                ForEach(ContributorVerdict.allCases, id: \.rawValue) { option in
                    verdictOption(option)
                }
                Spacer(minLength: 0)
            }
            .padding(TC.Space.xxs)
            .background(TC.surfaceInset, in: RoundedRectangle(cornerRadius: TC.Radius.card))
            Text(VerdictCopy.caption)
                .font(TC.Font_.captionSmall)
                .foregroundStyle(TC.inkTertiary)
                .fixedSize(horizontal: false, vertical: true)
                .multilineTextAlignment(.leading)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// One answer, styled as the tab strip's chips are -- selected means a
    /// raised surface and the green hairline, the same vocabulary this sheet
    /// already uses for "this one is current".
    private func verdictOption(_ option: ContributorVerdict) -> some View {
        let selected = verdict == option
        return Button {
            verdict = selected ? nil : option
            // A contributor who wrote a correction under `Failed` and then
            // answered `Worked` has withdrawn it. Clearing it here is what
            // stops text nobody can see any more from riding along on the
            // approval.
            if !correctionIsOffered {
                correction = ""
            }
        } label: {
            Text(option.label)
                .font(TC.Font_.caption.weight(selected ? .bold : .regular))
                .foregroundStyle(selected ? TC.inkPrimary : TC.inkSecondary)
                .padding(.horizontal, TC.Space.m)
                .padding(.vertical, TC.Space.control)
                .background {
                    RoundedRectangle(cornerRadius: TC.Radius.control)
                        .fill(selected ? TC.surface : Color.clear)
                }
                .overlay {
                    RoundedRectangle(cornerRadius: TC.Radius.control)
                        .strokeBorder(
                            selected ? TC.green.opacity(TC.Border.activeTabAlpha) : Color.clear,
                            lineWidth: TC.Border.hairline
                        )
                }
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? [.isSelected, .isButton] : .isButton)
    }

    private var gateStatement: some View {
        Text(ReadGate.statement)
            .font(TC.Font_.captionSmall)
            .foregroundStyle(TC.inkTertiary)
            .fixedSize(horizontal: false, vertical: true)
            .multilineTextAlignment(.leading)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func load() async {
        loading = true
        let outcome = await model.openPreview(entryID: entry.entryID)
        switch outcome {
        case .opened(let opened):
            preview = opened
            transcriptText = opened.body
            document = TranscriptDocument(opened.body)
            if let data = opened.summaryJSON.data(using: .utf8),
               let decoded = try? DaemonDecoding.decoder().decode(PreviewSummary.self, from: data)
            {
                summary = decoded
            } else {
                failure = "the summary could not be read"
            }
        case .failed(let message):
            failure = message
        }
        loading = false
    }

    private func closePreview() {
        preview?.close()
        preview = nil
    }
}

// MARK: - Sheet parts
//
// These are private to this file on purpose. The sheet is the only surface
// that draws an outlined sheet button at this size. The read-gate box that
// used to live here was the same drawing three screens had each written out,
// and it is now `TCReadGateCheckbox` in the design system.

/// The one size the spec states that the shared scale has no step for: the
/// sheet canvas (§4.6). The 5pt control padding and the 13pt read-gate box
/// that used to live here are now `TC.Space.control` and `TC.Control.checkbox`.
private enum SheetMetric {
    static let width: CGFloat = 760
    static let height: CGFloat = 620
}

/// True while the screenshot hook is rasterizing the shipping views.
///
/// `ImageRenderer` runs on the CPU with no window-server session, and two of
/// its limitations land squarely on this tab and are already documented
/// elsewhere in the shell: an NSView-backed `TextField` comes out as a solid
/// yellow bar with a "no entry" glyph (see `OnboardingConnectView`), and a
/// `ScrollView` comes out blank (see `ConsentScopesView`). Both are artifacts
/// of the renderer and neither is visible in the running app.
///
/// They still matter, because the captures are how this sheet is reviewed,
/// and a capture that shows a gold block where the search field is and an
/// empty space where the matched excerpt is says the opposite of what the
/// running app says -- the second one especially, since a match count with
/// no visible match is the one thing this tab must never do. So under the
/// hook, and only under the hook, the field is drawn rather than editable
/// and the scrolling regions lay out inline. Nothing about the shipping
/// behaviour changes: `TRACE_COMMONS_SCREENSHOT_DIR` is unset in a real run.
private enum CaptureMode {
    static let isRendering = DebugScreenshot.directory != nil
}

/// A scrolling region that lays its content out inline while the screenshot
/// hook is running, because `ImageRenderer` rasterizes a `ScrollView` as
/// blank. See `CaptureMode`.
private struct CaptureSafeScroll<Content: View>: View {
    @ViewBuilder let content: () -> Content

    init(@ViewBuilder content: @escaping () -> Content) {
        self.content = content
    }

    var body: some View {
        if CaptureMode.isRendering {
            content()
                .frame(maxWidth: .infinity, alignment: .leading)
                .clipped()
        } else {
            ScrollView { content() }
        }
    }
}

/// The sheet's own hairline. `Divider()` picks up the system separator
/// colour, which is a different grey from the one every card edge in this
/// app is drawn in.
private struct SheetHairline: View {
    var body: some View {
        Rectangle()
            .fill(TC.line)
            .frame(height: TC.Border.hairline)
    }
}

/// The outlined button of §6.1: a card face, a hairline, and the label in
/// ink. Used for every control in the sheet that is not Contribute.
private struct SheetSecondaryButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(TC.Font_.labelControl)
            .foregroundStyle(TC.inkPrimary)
            .padding(.horizontal, TC.Space.m)
            .padding(.vertical, TC.Space.control)
            .background(TC.surface, in: RoundedRectangle(cornerRadius: TC.Radius.control))
            .overlay {
                RoundedRectangle(cornerRadius: TC.Radius.control)
                    .strokeBorder(TC.line, lineWidth: TC.Border.hairline)
            }
            .opacity(configuration.isPressed ? 0.82 : 1)
            .contentShape(Rectangle())
    }
}

/// Wraps every occurrence of `term` in the gold highlight wash. SwiftUI can
/// carry a background colour on a run of an `AttributedString` but not the
/// 2pt radius and 2pt side padding the spec draws around it, so the wash is
/// flush against the glyphs.
private func highlighting(_ text: String, term: String) -> AttributedString {
    var attributed = AttributedString(text)
    guard !term.isEmpty else { return attributed }
    var searchRange = attributed.startIndex..<attributed.endIndex
    while let found = attributed[searchRange].range(of: term, options: .caseInsensitive) {
        attributed[found].backgroundColor = TC.goldHighlight
        attributed[found].foregroundColor = TC.inkPrimary
        searchRange = found.upperBound..<attributed.endIndex
    }
    return attributed
}

// MARK: - Tabs

/// The highest-value affordance in the product: type a client name, get
/// `0 matches` or jump-to-context, without reading 148 turns.
struct SearchTab: View {
    /// The body, already cut into chunks and holding its own bytes. Context
    /// snippets are cut from those bytes at the offsets the ABI reports.
    let document: TranscriptDocument?
    let preview: TCPreview?

    @State private var needle: String
    @State private var offsets: [Int]?
    @State private var searched: Bool
    @State private var recents: [String] = RecentSearches.load()
    @FocusState private var focused: Bool

    init(
        document: TranscriptDocument?,
        preview: TCPreview?,
        initialNeedle: String = "",
        initialOffsets: [Int]? = nil
    ) {
        self.document = document
        self.preview = preview
        _needle = State(initialValue: initialNeedle)
        _offsets = State(initialValue: initialOffsets)
        _searched = State(initialValue: initialOffsets != nil)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.m) {
            Text("Search this trace for anything you need to be sure isn't in it.")
                .font(TC.Font_.body)
                .foregroundStyle(TC.inkPrimary)

            HStack(spacing: TC.Space.s) {
                searchField
                Button("Search", action: run)
                    .buttonStyle(SheetSecondaryButtonStyle())
            }

            if !recents.isEmpty {
                // The contributor's own previous questions, one click away.
                HStack(spacing: TC.Space.s) {
                    Text("Recent:")
                        .font(TC.Font_.caption)
                        .foregroundStyle(TC.inkSecondary)
                    ForEach(recents, id: \.self) { term in
                        Button(term) { needle = term }
                            .buttonStyle(.plain)
                            .font(TC.Font_.caption)
                            .foregroundStyle(TC.greenText)
                    }
                }
            }

            resultSummary

            CaptureSafeScroll {
                VStack(alignment: .leading, spacing: TC.Space.sm) {
                    ForEach(Array(contexts.enumerated()), id: \.offset) { _, snippet in
                        Text(highlighting(snippet, term: needle))
                            .font(TC.Font_.monoCode)
                            .lineSpacing(TC.Font_.LineHeight.spacing(for: 11, TC.Font_.LineHeight.caption))
                            .textSelection(.enabled)
                            .padding(.horizontal, TC.Space.sm)
                            .padding(.vertical, TC.Space.s)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(
                                TC.surfaceScrim,
                                in: RoundedRectangle(cornerRadius: TC.Radius.control)
                            )
                    }
                }
            }
        }
        .onAppear { focused = true }
    }

    /// The spec's field: card face, hairline, radius 6, `5 x 10`.
    ///
    /// Under the screenshot hook it is drawn rather than editable -- see
    /// `CaptureMode`. The box, the type and the text are identical either
    /// way; what the capture loses is the caret and the ability to type,
    /// neither of which a still image was ever going to show.
    @ViewBuilder
    private var searchField: some View {
        Group {
            if CaptureMode.isRendering {
                Text(needle.isEmpty ? "Client name, hostname, anything" : needle)
                    .font(TC.Font_.body)
                    .foregroundStyle(needle.isEmpty ? TC.inkTertiary : TC.inkPrimary)
                    .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                TextField("Client name, hostname, anything", text: $needle)
                    .textFieldStyle(.plain)
                    .font(TC.Font_.body)
                    .foregroundStyle(TC.inkPrimary)
                    .focused($focused)
                    .onSubmit(run)
                    .onChange(of: needle) { _, _ in run() }
            }
        }
        .padding(.horizontal, TC.Space.sm)
        .padding(.vertical, TC.Space.control)
        .background(TC.surface, in: RoundedRectangle(cornerRadius: TC.Radius.control))
        .overlay {
            RoundedRectangle(cornerRadius: TC.Radius.control)
                .strokeBorder(TC.line, lineWidth: TC.Border.hairline)
        }
    }

    @ViewBuilder
    private var resultSummary: some View {
        if !searched || needle.isEmpty {
            Text("Type to search. Nothing is sent while you look.")
                .font(TC.Font_.body)
                .foregroundStyle(TC.inkSecondary)
        } else if offsets == nil {
            Text("The search couldn't run on this trace.")
                .font(TC.Font_.body)
                .foregroundStyle(TC.inkSecondary)
        } else if offsets!.isEmpty {
            // The answer to the only question this tab exists for, in
            // the app's two loudest tones -- each with a glyph, because a
            // green word and an amber word are the same word in greyscale.
            Label("0 matches", systemImage: TC.Tone.clear.symbol)
                .font(TC.Font_.headingAlert)
                .foregroundStyle(TC.Tone.clear.textColor)
        } else {
            Label("^[\(offsets!.count) match](inflect: true)", systemImage: TC.Tone.attention.symbol)
                .font(TC.Font_.headingAlert)
                .foregroundStyle(TC.Tone.attention.textColor)
        }
    }

    /// Runs on the main actor deliberately: the scan is a local in-memory
    /// pass, and keeping every touch of the `tc_preview*` on one thread is
    /// what the header's ownership rules ask for -- its pointer check narrows
    /// accidental misuse to an error, it does not make concurrent use safe.
    private func run() {
        searched = true
        guard !needle.isEmpty, let preview else {
            offsets = []
            return
        }
        offsets = preview.search(needle)
        if let offsets, !offsets.isEmpty {
            recents = RecentSearches.remember(needle)
        }
    }

    /// The ABI reports UTF-8 BYTE offsets, so context is cut from the
    /// document's bytes at those offsets, never from Swift's character
    /// indices.
    ///
    /// Bounded on both sides: at most 20 snippets, each at most a match plus
    /// 240 bytes of surroundings, so what this tab lays out does not grow
    /// with the trace. The whole-body walk that used to be here -- a fresh
    /// `Array(transcript.utf8)` every time this property was read, which is
    /// every keystroke -- is gone; the copy is made once, when the sheet
    /// builds its `TranscriptDocument`.
    ///
    /// The search itself is not bounded here and does not need to be: it
    /// runs in the daemon over the raw body and returns offsets, so no part
    /// of finding a match is text layout.
    private var contexts: [String] {
        guard let offsets, !offsets.isEmpty, let document else { return [] }
        return offsets.prefix(20).map { offset in
            let snippet = document.snippet(
                around: offset, matchBytes: needle.utf8.count, window: 120)
            guard !snippet.text.isEmpty else { return "" }
            let text = snippet.text.replacingOccurrences(of: "\n", with: " ")
            return (snippet.elidedBefore ? "…" : "") + text + (snippet.elidedAfter ? "…" : "")
        }
    }
}

struct WhatsInItTab: View {
    let entry: QueueEntry
    let summary: PreviewSummary

    var body: some View {
        CaptureSafeScroll {
            VStack(alignment: .leading, spacing: TC.Space.sm) {
                LabeledContent("Agent", value: entry.agentName)
                LabeledContent("Project", value: entry.projectLabel)
                LabeledContent("Turns recorded", value: "\(summary.eventCount)")
                LabeledContent("Session on disk", value: Format.bytes(summary.rawSessionBytes))
                LabeledContent("Would send", value: Format.bytes(summary.wouldSendBytes))
                Text("""
                "Would send" is usually larger than the file on disk: a redacted \
                envelope also carries schema, consent and privacy metadata the raw \
                session file does not.
                """)
                .font(TC.Font_.caption)
                .foregroundStyle(TC.inkSecondary)

                TCSectionHeader(title: "What scrubbing removed")
                    .padding(.top, TC.Space.xs)
                if summary.redactions.isEmpty {
                    // The one card in this tab that is drawn to be found: a
                    // session where no pattern fired is the session most
                    // worth a second look, and it is the case a count of
                    // removals cannot state.
                    HStack(alignment: .top, spacing: TC.Space.m) {
                        Image(systemName: TC.Tone.attention.symbol)
                            .font(.system(size: 14))
                            .foregroundStyle(TC.Tone.attention.color)
                            .accessibilityHidden(true)
                        Text("""
                        Nothing matched. On a session that touched credentials, that is \
                        itself worth a second look.
                        """)
                        .font(TC.Font_.body)
                        .foregroundStyle(TC.inkPrimary)
                        .fixedSize(horizontal: false, vertical: true)
                        Spacer(minLength: 0)
                    }
                    .padding(.horizontal, TC.Space.md)
                    .padding(.vertical, TC.Space.m)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .tcCard(emphasised: true)
                    .accessibilityElement(children: .combine)
                } else {
                    ForEach(summary.redactions.sorted(by: { $0.key < $1.key }), id: \.key) { kind, count in
                        Text("\(count) × \(kind.replacingOccurrences(of: "_", with: " "))")
                            .font(TC.Font_.body)
                            .foregroundStyle(TC.inkPrimary)
                    }
                }

                if !summary.piiLabelsPresent.isEmpty {
                    TCSectionHeader(title: "Personal-information categories seen")
                        .padding(.top, TC.Space.xs)
                    Text(summary.piiLabelsPresent.joined(separator: ", "))
                        .font(TC.Font_.body)
                        .foregroundStyle(TC.inkPrimary)
                    Text("Categories only. The matched text is never reported here.")
                        .font(TC.Font_.caption)
                        .foregroundStyle(TC.inkSecondary)
                }

                TCSectionHeader(title: "Residual risk")
                    .padding(.top, TC.Space.xs)
                Text(summary.residualRisk.replacingOccurrences(of: "_", with: " "))
                    .font(TC.Font_.body)
                    .foregroundStyle(TC.inkPrimary)
                Text("""
                Files touched and tools invoked are not in this contract's preview \
                summary, so they are not shown rather than guessed at.
                """)
                .font(TC.Font_.caption)
                .foregroundStyle(TC.inkTertiary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

/// The redacted transcript exactly as it would be sent, set as flat
/// monospace text and deliberately not as chat bubbles: these are the
/// literal bytes an approval covers, not a conversation to be enjoyed.
///
/// Redactions stay visible as inline chips rather than deletions, so a
/// contributor can see WHERE scrubbing fired -- which is the point. A hole
/// tells you nothing; a chip tells you the pipeline was standing there.
///
/// **All of the body is here.** It used to be the first 64 KB with a notice
/// saying the rest was not displayed, because one text run of a 17.5 MB
/// session pinned the main thread inside CoreText and took 2.97 GB to do
/// it. The body is now cut into chunks by `TranscriptDocument`; only the
/// chunks near the viewport are typeset, and chunks that scroll away are
/// dropped. What is bounded is glyph storage, not reach:
/// `TranscriptPaging.retainedLimitBytes` of text is laid out at any moment
/// whether the trace is 200 KB or 17.5 MB.
///
/// Two consequences a reader can see. Text selection is per block rather
/// than across the whole body -- a chunk that is not typeset has nothing to
/// select -- which is why "Copy everything" is here and copies all of it.
/// And the scrollbar settles by a row or two as chunks materialise, because
/// a chunk that is not laid out holds its place by an estimate.
struct TranscriptTab: View {
    let document: TranscriptDocument
    /// The chunks that are typeset right now, and the eviction that keeps
    /// that set under the ceiling. The policy lives in `TCShellCore` so it
    /// can be asserted against real byte counts without a running app.
    @State private var resident = TranscriptResidentChunks<AttributedString>()
    /// Where each chunk sits vertically, so a chunk that is not typeset
    /// still holds its place in the scroll.
    @State private var rows: TranscriptRowIndex?
    /// The last chunk to come into view; the window is centred on it, so
    /// overscan follows the reader in whichever direction they are going.
    @State private var anchor = 0
    @State private var columns = 0
    @State private var copied = false

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.sm) {
            Text(TranscriptMarkers.chipped(Self.caption, font: TC.Font_.caption))
                .font(TC.Font_.caption)
                .lineSpacing(TC.Font_.LineHeight.spacing(for: 11, TC.Font_.LineHeight.caption))
                .foregroundStyle(TC.inkSecondary)
                .fixedSize(horizontal: false, vertical: true)

            HStack(spacing: TC.Space.s) {
                Text("\(Format.bytes(document.totalBytes)), all of it.")
                    .font(TC.Font_.caption)
                    .foregroundStyle(TC.inkSecondary)
                Spacer(minLength: 0)
                Button(copied ? "Copied" : "Copy everything", action: copyAll)
                    .buttonStyle(SheetSecondaryButtonStyle())
                    .help(
                        "Puts the whole redacted body on the clipboard. "
                            + "Selection inside the transcript covers one block at a time."
                    )
                    .accessibilityIdentifier("transcript-copy-all")
            }

            GeometryReader { geometry in
                CaptureSafeScroll {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(laidOutIndices, id: \.self) { index in
                            chunkRow(index)
                        }
                    }
                    .padding(.horizontal, TC.Space.md)
                    .padding(.vertical, TC.Space.m)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .tcCard()
                .onAppear { measure(width: geometry.size.width) }
                .onChange(of: geometry.size.width) { _, width in measure(width: width) }
            }
        }
    }

    /// Which chunks exist as views at all.
    ///
    /// Every chunk, normally: a `LazyVStack` builds only the rows near the
    /// viewport, and the rest cost nothing until they are approached. Under
    /// the screenshot hook `CaptureSafeScroll` lays its content out inline
    /// with no viewport to be near, so there the list is cut to the resident
    /// window -- a capture of the first screen, which is what a capture
    /// shows anyway.
    private var laidOutIndices: Range<Int> {
        guard CaptureMode.isRendering else { return 0..<document.chunkCount }
        return TranscriptResidency.window(document, visible: 0..<1)
    }

    @ViewBuilder
    private func chunkRow(_ index: Int) -> some View {
        Group {
            if let text = resident.rendered[index] {
                Text(text)
                    .font(TC.Font_.monoTranscript)
                    .lineSpacing(
                        TC.Font_.LineHeight.spacing(for: 11, TC.Font_.LineHeight.transcript)
                    )
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                // Holds the chunk's place so the scroll extent is the whole
                // body's, not the resident window's.
                Color.clear
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .frame(height: placeholderHeight(index))
            }
        }
        .onAppear {
            anchor = index
            refresh()
        }
    }

    /// Moves the resident window to sit around `anchor`, typesetting what
    /// came into it and dropping what fell out of it.
    ///
    /// Only chunks that are new to the window are chipped and typeset, so a
    /// scroll of one chunk costs one chunk of layout -- measured at 6.4 ms
    /// for 4 KB with chips, inside a 16.7 ms frame.
    private func refresh() {
        let index = anchor
        resident.update(document: document, visible: index..<(index + 1)) { chunk in
            TranscriptMarkers.chipped(document.text(of: chunk), font: TC.Font_.monoTranscript)
        }
    }

    private func measure(width: CGFloat) {
        let usable = max(1, width - 2 * TC.Space.md)
        let next = max(1, Int(usable / Self.columnWidth))
        guard next != columns else { return }
        columns = next
        rows = TranscriptRowIndex(document, columns: next)
        refresh()
    }

    private func placeholderHeight(_ index: Int) -> CGFloat {
        let count = rows?.rows(of: index) ?? max(1, document.chunks[index].lineCount)
        return CGFloat(count) * Self.rowHeight
    }

    private func copyAll() {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(document.wholeText(), forType: .string)
        copied = true
    }

    // MARK: - Metrics
    //
    // Taken from the font rather than assumed, because the placeholder for a
    // chunk that is not laid out is only honest if it is the height that
    // chunk will have once it is.

    private static let font = NSFont.monospacedSystemFont(
        ofSize: NSFont.preferredFont(forTextStyle: .subheadline).pointSize,
        weight: .regular
    )
    private static let columnWidth = ("M" as NSString).size(withAttributes: [.font: font]).width
    private static let rowHeight =
        NSLayoutManager().defaultLineHeight(for: font)
        + TC.Font_.LineHeight.spacing(for: 11, TC.Font_.LineHeight.transcript)

    /// Spec copy, with the sample marker rendered as a live chip so the
    /// sentence demonstrates the thing it describes.
    private static let caption = """
        These are the exact bytes an approval covers. Marks like <PRIVATE_SECRET_1> \
        show where scrubbing fired — legible as chips, not holes.
        """
}

/// Turns the redaction pipeline's `<PRIVATE_*>` and `[REDACTED*]` markers
/// into chips: bold, on the measured chip pair rather than the gold ramp,
/// so they read as objects placed in the text instead of damage done to it.
///
/// Runs per chunk now, never over the whole body. The scan itself is in
/// `TranscriptMarkerScan` and is shared with the chunker, which uses it to
/// avoid cutting through a marker -- half a marker rendered as body text in
/// one block and the other half in the next would read as content that was
/// never scrubbed.
private enum TranscriptMarkers {
    static func chipped(_ text: String, font: Font) -> AttributedString {
        var out = AttributedString()
        var cursor = text.startIndex
        for range in TranscriptMarkerScan.spans(in: text) {
            out.append(AttributedString(String(text[cursor..<range.lowerBound])))
            var chip = AttributedString(String(text[range]))
            chip.font = font.weight(.bold)
            chip.backgroundColor = TC.redactionChipBackground
            chip.foregroundColor = TC.redactionChipForeground
            out.append(chip)
            cursor = range.upperBound
        }
        out.append(AttributedString(String(text[cursor...])))
        return out
    }
}

/// The consent scopes this upload will carry, restated at the moment of
/// consent rather than only at onboarding.
struct PermissionsTab: View {
    let summary: PreviewSummary
    let options: [ConsentScope]

    var body: some View {
        CaptureSafeScroll {
            VStack(alignment: .leading, spacing: TC.Space.m) {
                TCSectionHeader(title: "What this upload asks for")
                ForEach(summary.consentScopes, id: \.self) { scope in
                    VStack(alignment: .leading, spacing: TC.Space.micro) {
                        Text(ScopeCopy.title(for: scope, options: options))
                            .font(TC.Font_.cardTitle)
                            .foregroundStyle(TC.inkPrimary)
                        if let description = options.first(where: { $0.name == scope })?.description {
                            Text(description)
                                .font(TC.Font_.caption)
                                .foregroundStyle(TC.inkSecondary)
                        }
                    }
                }
                Text("""
                These are the permissions this device requests. Trace Commons can \
                narrow them, never widen them -- and if your permissions change \
                between now and sending, this approval stops applying and you are \
                asked again.
                """)
                .font(TC.Font_.caption)
                .foregroundStyle(TC.inkSecondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

enum ScopeCopy {
    /// The first words of each label carry the distinction, because that is
    /// all most people read.
    static func title(for wireName: String, options: [ConsentScope]) -> String {
        switch wireName {
        case "debugging_evaluation": return "Finding bugs and measuring agents"
        case "benchmark_only", "benchmark_creation": return "Turn my traces into test cases"
        case "ranking_training", "reward_model_training":
            return "Train models that judge agent output"
        case "model_training": return "Train coding models directly"
        case "public_attribution": return "List my handle publicly as a contributor"
        default:
            return options.first(where: { $0.name == wireName })?.name
                ?? wireName.replacingOccurrences(of: "_", with: " ")
        }
    }
}

/// Recent searches, kept for the life of the process and never written to
/// disk.
///
/// They used to persist in `UserDefaults`, on the reasoning that the second
/// trace should be one keystroke. That reasoning is sound; the storage was
/// not. A recent-search list is the contributor's list of the things they
/// were afraid of leaking -- a client name, an employer, an unreleased
/// product, the name of a person. It is assembled precisely because those
/// strings are sensitive, which makes it a worse thing to leave on disk
/// than most of what the search was checking for.
///
/// Nothing else in this product writes that class of string down. The Linux
/// and Windows shells both hold theirs in memory for the same reason, so
/// this is also what makes the three agree.
///
/// In memory it still does its job: a contributor checking six traces in one
/// sitting types each term once. It only stops helping across a restart,
/// which is the trade being made deliberately.
enum RecentSearches {
    private static let legacyKey = "trace-commons.recent-searches"

    private static var terms: [String] = []

    static func load() -> [String] {
        // Earlier builds wrote this list to disk. Stopping the writes does
        // not unwrite what they already stored, and an install that has been
        // upgraded would otherwise keep those terms indefinitely with no
        // surface left in the app to clear them. So the key is removed the
        // first time this is read, rather than merely ignored.
        purgeLegacyStore()
        return terms
    }

    static func remember(_ term: String) -> [String] {
        terms = [term] + terms.filter { $0 != term }
        terms = Array(terms.prefix(6))
        return terms
    }

    /// Removes the old on-disk list. Idempotent, and safe when absent.
    static func purgeLegacyStore() {
        guard UserDefaults.standard.object(forKey: legacyKey) != nil else {
            return
        }
        UserDefaults.standard.removeObject(forKey: legacyKey)
    }
}
