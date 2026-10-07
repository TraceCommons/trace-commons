import AppKit
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// "Look inside": the one surface in the product that deliberately shows
/// trace content, because consent to send something you cannot see is not
/// consent.
///
/// **Read-only** (#1241, Ron's #1146 `PreviewInspector`). The inspector's
/// session card is the review: it carries the verdict, the correction and
/// the product's one approve control. This sheet shows what would be sent
/// and offers the native review actions; its footer is Close and nothing
/// else. It used to carry Contribute, Not this one, the verdict and the
/// correction too, which made two approve controls for one session.
///
/// Ron's order, top to bottom: the eyebrow, title and description, the
/// gate statement, what would be sent against what is on disk, the
/// per-session send disclosure, the native review (Prepare admission, and
/// Request witness review whenever the session supports one -- up front,
/// not only after a preview failed), then the tabs: Exactly what would be
/// sent (a page at a time, with turn separators), Search original (a
/// count), Turn index. Native's search of the redacted text stays beside
/// them, with Command-F, and so does Copy everything.
///
/// Every word on it that is Ron's comes from the core
/// (`MonitorLookInsideCopy`).
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
    /// How the preview closes when it is raised in a `GlassModal`
    /// (`PreviewModal`); nil in a stock sheet, which the environment's
    /// dismiss closes.
    let onClose: (() -> Void)?

    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var witnessSupported = false
    @State private var confirmingWitness = false
    @State private var preparingAdmission = false
    @State private var witnessRequested = false
    @State private var witnessWorking = false
    @State private var preview: TCPreview?
    @State private var summary: PreviewSummary?
    @State private var transcriptText: String
    /// The transcript cut into chunks, built once when the body arrives.
    ///
    /// One document serves every tab that walks the body: the transcript
    /// tab pages through its chunks, and the redacted search cuts its
    /// context snippets out of its bytes.
    @State private var document: TranscriptDocument?
    /// `LookInside.bodyDigest` of the body on screen: the anchor the turn
    /// index is asked for and checked against.
    @State private var digest: String?
    /// The core's turn index for that body, once asked for. Nil is "not
    /// asked" or "refused", never an empty index.
    @State private var turns: PreviewTurns?
    @State private var turnsFailed = false
    @State private var loadingTurns = false
    /// Pages of the body shown so far (Ron's Load more).
    @State private var pages = 1
    @State private var failure: String?
    /// The daemon's sentence for a review it refused.
    ///
    /// Separate from `failure`, which also holds messages from the local
    /// preview build -- including raw error text. Only a refusal the daemon
    /// classified lands here, so the notice below can prefer it without
    /// risking an internal string reaching a screen.
    @State private var witnessRefusal: String?
    /// Set when a review met a busy witness: the time it may be tried again.
    @State private var witnessBusyRetry: String?
    @State private var loading: Bool

    /// Ron's first tab is what would be sent.
    @State private var tab: Tab = .transcript

    enum Tab: String, CaseIterable, Identifiable {
        case transcript, searchOriginal, turnIndex, search
        var id: String { rawValue }

        /// Ron's three tabs in the core's words; native's redacted search
        /// keeps its own one-word label.
        func title(_ words: MonitorLookInsideCopy?) -> String {
            switch self {
            case .transcript: return words?.title ?? ""
            case .searchOriginal: return words?.searchOriginal ?? ""
            case .turnIndex: return words?.turnIndex ?? ""
            case .search: return "Search"
            }
        }
    }

    /// How much of the body one Load more adds: the transcript's resident
    /// ceiling, so a page is never more than the tab keeps typeset.
    static let pageBytes = TranscriptPaging.retainedLimitBytes

    init(entry: QueueEntry, preloaded: Preloaded? = nil, onClose: (() -> Void)? = nil) {
        self.entry = entry
        self.preloaded = preloaded
        self.onClose = onClose
        _summary = State(initialValue: preloaded?.summary)
        _transcriptText = State(initialValue: preloaded?.transcript ?? "")
        _document = State(initialValue: preloaded.map { TranscriptDocument($0.transcript) })
        _digest = State(initialValue: preloaded.map { LookInside.bodyDigest($0.transcript) })
        _loading = State(initialValue: preloaded == nil)
    }

    /// Bumped by Command-F. `SearchTab` watches it and takes focus, so the
    /// shortcut works from any tab and from anywhere on the search tab.
    @State private var searchFocusRequest = 0

    /// The Monitor's Traces words, decoded once.
    private static let traces = MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON())

    /// Ron's Look-inside words, or nil when the core's table did not
    /// decode -- in which case the sheet draws no words of Ron's rather
    /// than any of its own.
    private var words: MonitorLookInsideCopy? { Self.traces?.lookInside }

    /// Ron's Look-inside title: what `PreviewModal` titles the modal with,
    /// in place of the header's own when the sheet is raised in one.
    static var modalTitle: String? { traces?.lookInside.title }

    /// The core's other Close, for a sheet whose Look-inside table did not
    /// decode: the footer's one control is never drawn without a name.
    private static let fallbackClose = TCCoreCopy.firstRunCopyJSON().flatMap(FirstRunCopy.decode)?.passkey.close

    /// Close in the core's words: the Look-inside table's, else the other.
    private var closeWord: String? { words?.close ?? Self.fallbackClose }

    /// The one sentence this sheet says when it cannot show the session:
    /// the core's (`session_review.cannot_show_title`), the same the
    /// session card reads.
    private static var cannotShow: String {
        Self.traces?.sessionReview.cannotShowTitle ?? Self.cannotShowFallback
    }

    /// The core's `cannot_show_title`, verbatim. Read only when the core's
    /// table does not decode, so the notice is never drawn without a title
    /// (the `HealthCopy.onHoldFallback` precedent).
    static let cannotShowFallback = "This one can't be shown."

    /// The core's line under that sentence, or nothing when its table did
    /// not decode. Every failure that is not a witness refusal reads this:
    /// `failure` can hold the local preview build's raw error text.
    private var cannotShowDetail: String { Self.traces?.sessionReview.cannotShowBody ?? "" }

    /// Whether the session holds a certificate now. `entry` is the copy the
    /// sheet opened with, and a witness review that succeeds while it is
    /// open earns one, so the live queue row is read, and the body the
    /// review reopened on.
    private var holdsCertificate: Bool {
        LookInside.holdsCertificate(
            liveRow: (model.awaitingDecision.first(where: { $0.entryID == entry.entryID }) ?? entry).holdsCertificate,
            envelopeDigest: summary?.envelopeDigest)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            Divider().overlay(GlassColor.hairline)
            content
            Divider().overlay(GlassColor.hairline)
            footer
        }
        // The spec's canvas is the floor, not the fixed size: the transcript
        // and search tabs can use the additional reading space. Ideal
        // keeps the first presentation at
        // the spec measure and the screenshot hook renders at exactly it.
        // In a modal the window sets the floor instead.
        .frame(
            minWidth: onClose == nil ? SheetMetric.width : nil, idealWidth: SheetMetric.width, maxWidth: .infinity,
            minHeight: onClose == nil ? SheetMetric.height : nil, idealHeight: SheetMetric.height,
            maxHeight: .infinity
        )
        .background {
            // A shortcut needs a control to hang from. This one is never
            // seen and never focused; it exists so Command-F does what it
            // does in every other macOS window: go to the search field.
            Button("") {
                tab = .search
                searchFocusRequest += 1
            }
            .keyboardShortcut("f", modifiers: .command)
            .buttonStyle(.plain)
            .opacity(0)
            .frame(width: 0, height: 0)
            .accessibilityHidden(true)
            .focusable(false)
            // With no Close word from the core the footer draws no control,
            // so Escape hangs from an unseen one here instead.
            if closeWord == nil {
                Button("") { close() }
                    .keyboardShortcut(.cancelAction)
                    .buttonStyle(.plain)
                    .opacity(0)
                    .frame(width: 0, height: 0)
                    .accessibilityHidden(true)
                    .focusable(false)
            }
        }
        // In a modal the modal is the pane; a second tier would be glass on
        // glass.
        .modifier(PreviewChrome(inModal: onClose != nil))
        .task(id: entry.entryID) {
            guard preloaded == nil else { return }
            witnessSupported = await model.supportsWitnessReview()
            await load()
        }
        .onDisappear { closePreview() }
        .glassModal(isPresented: $confirmingWitness) {
            if let copy = model.witnessCopy?.review {
                WitnessReviewConsent(copy: copy, confirmLine: words?.witnessConfirmLine,
                                     confirmLabel: words?.witnessConfirmLabel, onCancel: { confirmingWitness = false }) {
                    Task { await prepareWitness() }
                }
            }
        }
    }

    /// Closes the preview: the modal's close when it is raised in one, the
    /// sheet's dismiss otherwise.
    private func close() {
        if let onClose { onClose() } else { dismiss() }
    }

    // MARK: - Chrome

    /// Ron's head of the inspector: what this is, the gate statement, the
    /// sizes, the send disclosure, and the native review actions.
    private var header: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            if let words {
                // In a modal the modal draws the title (`PreviewModal`).
                if onClose == nil {
                    Text(words.eyebrow)
                        .glassType(GlassTokens.TypeScale.eyebrow)
                        .foregroundStyle(GlassColor.textTertiary)
                    Text(words.title)
                        .glassType(GlassTokens.TypeScale.title)
                        .foregroundStyle(GlassColor.textPrimary)
                }
                Text(words.description)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            gateStatement
            if let summary {
                sizes(summary)
                // K11: what leaves this computer for this session, before
                // and after redaction, and what its witness was checked
                // against.
                SessionSendDisclosureView(
                    entry: entry,
                    rawSessionBytes: summary.rawSessionBytes,
                    wouldSendBytes: summary.wouldSendBytes)
            }
            nativeReview
        }
        .padding(.horizontal, GlassTokens.Space.s9)
        .padding(.vertical, GlassTokens.Space.s8)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// Ron's quiet card: the session's tool and folder, and what would be
    /// sent against the file on disk.
    private func sizes(_ summary: PreviewSummary) -> some View {
        GlassCard(quiet: true) {
            HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
                Text(entry.agentName)
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .foregroundStyle(GlassColor.textPrimary)
                Text(entry.projectLabel)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                Spacer(minLength: GlassTokens.Space.s6)
                if let words {
                    Text(FirstRunCopy.fill(words.wouldSend, ["size": Format.bytes(summary.wouldSendBytes)])
                        + " · " + FirstRunCopy.fill(words.onDisk, ["size": Format.bytes(summary.rawSessionBytes)]))
                        .glassType(GlassTokens.TypeScale.caption)
                        .monospacedDigit()
                        .foregroundStyle(GlassColor.textSecondary)
                }
            }
            .accessibilityElement(children: .combine)
        }
    }

    /// Ron's `NativeReviewActions`: optional checks that stay local until
    /// confirmed. Drawn on every preview that has one to offer -- the
    /// witness review is no longer reached by a preview failing first --
    /// and never for a session that already holds a certificate.
    ///
    /// Prepare admission is still gated on the enrollment, and gated nowhere
    /// else: an invited contributor has no evidence-bearing path, so the
    /// control could only refuse them and is absent instead.
    @ViewBuilder
    private var nativeReview: some View {
        let admission = model.daemonSettings?.admissionEvidenceOffered == true
        let holdsCertificate = self.holdsCertificate
        let witness = LookInside.offersWitnessReview(
            supported: witnessSupported, witnessPinned: model.witnessStateCode == 1,
            holdsCertificate: holdsCertificate)
        if let words,
           LookInside.showsNativeReview(
               admissionOffered: admission, offersWitness: witness, holdsCertificate: holdsCertificate)
        {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                Divider().overlay(GlassColor.hairline)
                Text(words.nativeReview)
                    .glassType(GlassTokens.TypeScale.eyebrow)
                    .foregroundStyle(GlassColor.textTertiary)
                Text(words.nativeReviewCaption)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: GlassTokens.Space.s4) {
                    if admission {
                        Button(words.prepareAdmission) { preparingAdmission = true }
                            .buttonStyle(GlassButtonStyle(.glass))
                    }
                    if witness {
                        Button(witnessWorking ? words.witnessReviewing : words.requestWitnessReview) {
                            confirmingWitness = true
                        }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .disabled(witnessWorking || model.witnessCopy?.review == nil)
                    }
                    Spacer(minLength: 0)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            // Over the whole window, stacked over the preview's own modal.
            .glassModal(isPresented: $preparingAdmission) {
                GlassModal(
                    title: words.prepareAdmission, width: .narrow,
                    actions: [.cancel(words.close) { preparingAdmission = false }],
                    onCancel: { preparingAdmission = false }
                ) {
                    GlassModalBody { AdmissionPreparationView(entryID: entry.entryID) }
                }
            }
        }
    }

    @ViewBuilder
    private var content: some View {
        if witnessWorking, let copy = model.witnessCopy?.review {
            SheetNotice(title: copy.heading, detail: copy.working)
        } else if loading {
            // Without the core's Look-inside table there is no loading line,
            // and the tabs will not draw: said now, not an empty notice.
            if let words {
                SheetNotice(title: nil, detail: words.loadingTranscript)
            } else {
                SheetNotice(title: Self.cannotShow, detail: cannotShowDetail)
            }
        } else if failure != nil {
            // A refusal the daemon classified wins; otherwise the one fixed
            // sentence over the core's cannot-show line, which is also what
            // a failure that is not a refusal gets -- `failure` can hold raw
            // local error text and never reaches a screen.
            if witnessRequested, let retry = witnessBusyRetry {
                // A busy witness judged nothing: not a refusal. The
                // daemon's busy sentence, and when to try again.
                SheetNotice(
                    title: model.witnessCopy?.review?.heading,
                    detail: [witnessRefusal ?? model.witnessCopy?.review?.failed ?? cannotShowDetail, retry]
                        .joined(separator: "\n")
                )
            } else {
                SheetNotice(
                    title: Self.cannotShow,
                    detail: witnessRequested
                        ? (witnessRefusal ?? model.witnessCopy?.review?.failed ?? cannotShowDetail)
                        : cannotShowDetail
                )
            }
        } else if summary != nil, let words, let document {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
                tabBar(words)
                switch tab {
                case .transcript:
                    TranscriptTab(
                        document: document,
                        words: words,
                        turns: turns?.turns ?? [],
                        shownChunks: shownChunks(document),
                        onLoadMore: { pages += 1 },
                        onAddSeparators: fullyShown(document) && turns == nil && !loadingTurns
                            ? { Task { await loadTurns() } } : nil
                    )
                    .id(turns?.turnCount ?? -1)
                case .searchOriginal:
                    OriginalSearchTab(words: words) { needle in
                        model.searchOriginal(entryID: entry.entryID, needle: needle)
                    }
                case .turnIndex:
                    turnIndexTab(words, document: document)
                case .search:
                    SearchTab(
                        document: document,
                        preview: preview,
                        searchOriginal: { needle in
                            model.searchOriginal(entryID: entry.entryID, needle: needle)
                        },
                        initialNeedle: preloaded?.needle ?? "",
                        initialOffsets: preloaded?.offsets,
                        focusRequest: searchFocusRequest
                    )
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, GlassTokens.Space.s9)
            .padding(.vertical, GlassTokens.Space.s8)
        } else if summary != nil {
            // A summary with no body to show, or the core's Look-inside
            // table would not decode: said, never a blank pane or a row of
            // unnamed tabs.
            SheetNotice(title: Self.cannotShow, detail: cannotShowDetail)
        }
    }

    /// The core's word for the review this sheet is: the modal's title when
    /// the Look-inside table does not decode, so the modal is never
    /// untitled.
    static let reviewWord = MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON())?.review

    /// The tabs, in Ron's order, with native's redacted search after them.
    private func tabBar(_ words: MonitorLookInsideCopy) -> some View {
        GlassSegmentedTabs(tab.title(words), selection: $tab,
                           segments: Tab.allCases.map { item in GlassSegment(item.title(words), value: item) })
    }

    private func shownChunks(_ document: TranscriptDocument) -> Int {
        LookInside.shownChunks(document, pages: pages, pageBytes: Self.pageBytes)
    }

    /// Ron's `loaded`: every page of the body is on screen.
    private func fullyShown(_ document: TranscriptDocument) -> Bool {
        LookInside.remainingBytes(document, shownChunks: shownChunks(document)) == 0
    }

    /// Ron's Turn index tab: once the whole body has been shown, the core's
    /// index over it, one row per turn.
    @ViewBuilder
    private func turnIndexTab(_ words: MonitorLookInsideCopy, document: TranscriptDocument) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            if !fullyShown(document) {
                Text(words.turnsNeedFullRead)
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textTertiary)
            } else if turns == nil {
                Button(words.loadTurnIndex) { Task { await loadTurns() } }
                    .buttonStyle(GlassButtonStyle(.primary))
                    .disabled(loadingTurns)
            }
            if turnsFailed, let line = Self.traces?.requestFailed {
                Text(line)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
            }
            if let turns, !turns.turns.isEmpty {
                Text(words.turnIndexEyebrow)
                    .glassType(GlassTokens.TypeScale.eyebrow)
                    .foregroundStyle(GlassColor.textTertiary)
                CaptureSafeScroll {
                    LazyVStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                        ForEach(turns.turns, id: \.index) { turn in
                            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                                Text(LookInside.turnTitle(turn))
                                    .glassType(GlassTokens.TypeScale.bodyStrong)
                                    .foregroundStyle(GlassColor.textPrimary)
                                Text(LookInside.turnDetail(turn, words: words))
                                    .glassType(GlassTokens.TypeScale.caption)
                                    .monospacedDigit()
                                    .foregroundStyle(GlassColor.textSecondary)
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .accessibilityElement(children: .combine)
                        }
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// Close, and nothing else: this sheet approves nothing. The one
    /// approve control is the inspector card's. Escape closes it; Return is
    /// bound to nothing.
    private var footer: some View {
        HStack(spacing: GlassTokens.Space.s4) {
            Spacer(minLength: 0)
            // Never an unnamed control: with no Close word from the core
            // there is no visible control, and Escape still closes (an
            // unseen control in the sheet's background carries it).
            if let closeLabel = closeWord {
                Button(closeLabel) { close() }
                    .buttonStyle(GlassButtonStyle(.glass))
                    .keyboardShortcut(.cancelAction)
            }
        }
        .padding(.horizontal, GlassTokens.Space.s9)
        .padding(.vertical, GlassTokens.Space.s8)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    // MARK: - The gate statement

    /// The consent surface's sentences, read once. Nil if the payload did
    /// not arrive or would not parse, in which case the sheet shows no claim
    /// rather than a blank one -- see `ConsentCopy.decode`.
    private var consent: ConsentCopy? {
        TCConsentCopy.copyJSON().flatMap(ConsentCopy.decode(fromJSON:))
    }

    private var gateStatement: some View {
        Text(consent?.gateStatement ?? "")
            .glassType(GlassTokens.TypeScale.caption)
            .foregroundStyle(GlassColor.textTertiary)
            .fixedSize(horizontal: false, vertical: true)
            .multilineTextAlignment(.leading)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    // MARK: - Loading

    private func prepareWitness() async {
        guard witnessSupported, !witnessWorking else { return }
        witnessRequested = true
        witnessWorking = true
        summary = nil
        closePreview()
        let outcome = await model.witnessReviewOutcome(entryID: entry.entryID)
        witnessWorking = false
        if outcome.succeeded { await load() }
        else {
            // The daemon classifies the refusal and chooses the words; this
            // used to render one sentence for all fourteen causes, so a
            // receipt the reviewer declined read exactly like a reviewer that
            // was down. `review.failed` remains the fallback for a response
            // carrying no sentence -- a transport failure, or a daemon older
            // than this shell.
            witnessRefusal = outcome.sentence
            witnessBusyRetry = outcome.retryLine
            failure = outcome.sentence ?? model.witnessCopy?.review?.failed
            loading = false
        }
    }

    /// Asks the core for the turn index over the body on screen, and keeps
    /// it only if it indexes that body: offsets against another body would
    /// still look like a transcript.
    private func loadTurns() async {
        guard let digest, !loadingTurns else { return }
        loadingTurns = true
        turnsFailed = false
        let index = await model.previewTurns(entryID: entry.entryID, bodyDigest: digest)
        loadingTurns = false
        guard self.digest == digest else { return }
        if let index, index.indexes(bodyDigest: digest) {
            turns = index
        } else {
            turnsFailed = true
        }
    }

    private func load() async {
        loading = true
        failure = nil
        let outcome = await model.openPreview(entryID: entry.entryID)
        switch outcome {
        case .opened(let opened):
            preview = opened
            let body = opened.body
            transcriptText = body
            document = TranscriptDocument(body)
            // A witness review reopens the preview on a new body: its index,
            // its pages and its anchor start over.
            digest = LookInside.bodyDigest(body)
            turns = nil
            turnsFailed = false
            pages = 1
            if let data = opened.summaryJSON.data(using: .utf8),
               let decoded = try? DaemonDecoding.decoder().decode(PreviewSummary.self, from: data)
            {
                summary = decoded
                witnessRequested = decoded.envelopeDigest?.hasPrefix("witness-sha256:") == true
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

/// Ron's Search original tab: the original session is searched locally and
/// answers with a count, never with text.
private struct OriginalSearchTab: View {
    let words: MonitorLookInsideCopy
    /// How many times a term appears in the PRE-redaction session, or nil
    /// when that could not be checked -- which is never zero.
    let search: (String) -> Int?

    @State private var needle = ""
    @State private var matches: Int?
    @State private var failed = false

    private static let requestFailed = MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON())?.requestFailed

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            Text(words.searchCaption)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            Text(words.searchLabel)
                .glassType(GlassTokens.TypeScale.label)
                .foregroundStyle(GlassColor.textPrimary)
            HStack(spacing: GlassTokens.Space.s4) {
                // `GlassTextField`, as the redacted search's field; drawn
                // rather than editable under the screenshot hook.
                if CaptureMode.isRendering {
                    Text(needle.isEmpty ? words.searchPlaceholder : needle)
                        .glassType(GlassTokens.TypeScale.label.weight(.regular))
                        .foregroundStyle(needle.isEmpty ? GlassColor.textTertiary : GlassColor.textPrimary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 10)
                        .frame(minHeight: GlassTokens.Size.controlLarge)
                        .glassFieldWell(invalid: false)
                } else {
                    GlassTextField(words.searchLabel, text: $needle, prompt: words.searchPlaceholder, showsLabel: false)
                        .onSubmit(check)
                        .onChange(of: needle) { _, _ in
                            matches = nil
                            failed = false
                        }
                }
                Button(words.checkCount, action: check)
                    .buttonStyle(GlassButtonStyle(.glass))
                    .disabled(trimmed.isEmpty)
            }
            if let matches {
                Text(LookInside.originalMatches(matches, words: words))
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .monospacedDigit()
                    .foregroundStyle(GlassColor.textPrimary)
            } else if failed, let line = Self.requestFailed {
                Text(line)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var trimmed: String { needle.trimmingCharacters(in: .whitespacesAndNewlines) }

    private func check() {
        guard !trimmed.isEmpty else { return }
        let count = search(trimmed)
        matches = count
        failed = count == nil
    }
}


// MARK: - Sheet parts
//
// These are private to this file on purpose: only this sheet draws them.

/// The one size the spec states that the shared scale has no step for: the
/// sheet canvas (§4.6), used as the sheet's minimum and its first-shown
/// size.
private enum SheetMetric {
    static let width: CGFloat = 760
    static let height: CGFloat = 620
}

/// True while the screenshot hook is rasterizing the shipping views.
///
/// `ImageRenderer` runs on the CPU with no window-server session, and two of
/// its limitations land squarely on this tab and are already documented
/// elsewhere in the shell: an NSView-backed `TextField` comes out as a solid
/// yellow bar with a "no entry" glyph, and a `ScrollView` comes out blank.
/// Both are artifacts of the renderer and neither is visible in the running
/// app.
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

/// A state the content area holds instead of the tabs: working, loading, a
/// failure, a transcript not ready. The title carries the ask dot and its
/// words; the detail sits under it. Centred in the space the tabs would use.
/// With no title words there is no title, and so no dot without words.
private struct SheetNotice: View {
    let title: String?
    let detail: String

    var body: some View {
        GlassNotice(tone: .ask, title: title?.isEmpty == false ? title : nil) {
            Text(detail)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: 480)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(GlassTokens.Space.s9)
    }
}

/// Wraps every occurrence of `term` in the highlight wash: the ask colour at
/// 32% behind primary text. TCDesign has no highlight token and this adds
/// none (ruling R-13). SwiftUI can carry a background colour on a run of an
/// `AttributedString` but not a radius or side padding, so the wash is flush
/// against the glyphs.
private func highlighting(_ text: String, term: String) -> AttributedString {
    var attributed = AttributedString(text)
    guard !term.isEmpty else { return attributed }
    var searchRange = attributed.startIndex..<attributed.endIndex
    while let found = attributed[searchRange].range(of: term, options: .caseInsensitive) {
        attributed[found].backgroundColor = GlassTokens.Color.statusAsk.color.opacity(0.32)
        attributed[found].foregroundColor = GlassColor.textPrimary
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
    /// How many times a term appears in the PRE-redaction session, or nil
    /// when that could not be checked. A closure rather than a daemon
    /// reference because `AppModel` is the only thing in this app that talks
    /// to the daemon.
    let searchOriginal: (String) -> Int?
    /// Command-F, as a counter the sheet bumps. Any change puts the caret
    /// in the field; the value itself means nothing.
    let focusRequest: Int

    @State private var needle: String
    @State private var offsets: [Int]?
    @State private var outcome: OriginalSearchOutcome?
    @State private var searched: Bool
    @State private var recents: [String] = RecentSearches.load()
    @FocusState private var focused: Bool

    init(
        document: TranscriptDocument?,
        preview: TCPreview?,
        searchOriginal: @escaping (String) -> Int? = { _ in nil },
        initialNeedle: String = "",
        initialOffsets: [Int]? = nil,
        focusRequest: Int = 0
    ) {
        self.document = document
        self.preview = preview
        self.searchOriginal = searchOriginal
        self.focusRequest = focusRequest
        _needle = State(initialValue: initialNeedle)
        _offsets = State(initialValue: initialOffsets)
        _searched = State(initialValue: initialOffsets != nil)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s5) {
            Text("Search this trace for anything you need to be sure isn't in it.")
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textPrimary)

            HStack(spacing: GlassTokens.Space.s4) {
                searchField
                Button("Search", action: commit)
                    .buttonStyle(GlassButtonStyle(.glass))
            }

            if !recents.isEmpty {
                // The contributor's own previous questions, one click away.
                HStack(spacing: GlassTokens.Space.s4) {
                    Text("Recent:")
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                    ForEach(recents, id: \.self) { term in
                        Button(term) { needle = term }
                            .buttonStyle(GlassButtonStyle(.link))
                    }
                }
            }

            resultSummary

            CaptureSafeScroll {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                    ForEach(Array(contexts.enumerated()), id: \.offset) { _, snippet in
                        GlassCard(quiet: true) {
                            Text(highlighting(snippet, term: needle))
                                .glassType(GlassTokens.TypeScale.mono)
                                .foregroundStyle(GlassColor.textPrimary)
                                .textSelection(.enabled)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                    }
                }
            }
        }
        .onAppear { focused = true }
        .onChange(of: focusRequest) { _, _ in focused = true }
    }

    /// The field: `GlassTextField`, its focus bound for Command-F and its
    /// submit running the search.
    ///
    /// Under the screenshot hook it is drawn rather than editable -- see
    /// `CaptureMode`. The well, the type and the text are identical either
    /// way; what the capture loses is the caret and the ability to type,
    /// neither of which a still image was ever going to show.
    @ViewBuilder
    private var searchField: some View {
        let prompt = "Client name, hostname, anything"
        if CaptureMode.isRendering {
            Text(needle.isEmpty ? prompt : needle)
                .glassType(GlassTokens.TypeScale.label.weight(.regular))
                .foregroundStyle(needle.isEmpty ? GlassColor.textTertiary : GlassColor.textPrimary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 10)
                .frame(minHeight: GlassTokens.Size.controlLarge)
                .glassFieldWell(invalid: false)
        } else {
            GlassTextField(prompt, text: $needle, prompt: prompt, showsLabel: false, focus: $focused)
                .onSubmit(commit)
                .onChange(of: needle) { _, _ in run() }
        }
    }

    @ViewBuilder
    private var resultSummary: some View {
        if !searched || needle.isEmpty {
            Text("Type to search. Nothing is sent while you look.")
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textSecondary)
        } else if offsets == nil {
            Text("The search couldn't run on this trace.")
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textSecondary)
        } else if let outcome {
            // The answer to the only question this tab exists for, as a
            // dot with its sentence -- never the colour alone, because a
            // green dot and an amber dot are the same dot in greyscale.
            //
            // Which status is the outcome's to decide: a term that is still
            // in what would be sent is the one to slow down on, and a term
            // that was removed reads as clear even though the redacted body
            // and the original disagree about it.
            //
            // Three, not two. `.unknown` is a missing answer, and it used to
            // draw in the clear tone -- the app's all-clear glyph beside the
            // sentence that says the check did not run. See
            // `OriginalSearchOutcome.Emphasis`.
            GlassStatusLabel(outcome.sentence, status: Self.status(for: outcome.emphasis))
        } else if offsets!.isEmpty {
            // No outcome: the preloaded screenshot path, which sets offsets
            // without running a search.
            GlassStatusLabel("0 matches", status: .on)
        } else {
            GlassStatusLabel(Self.matchCount(offsets!.count), status: .ask)
        }
    }

    /// The status for an outcome's emphasis. Three of them, because "could
    /// not check" is neither a clean answer nor an alarming one.
    static func status(for emphasis: OriginalSearchOutcome.Emphasis) -> GlassStatus {
        switch emphasis {
        case .attention: return .ask
        case .clear: return .on
        case .unchecked: return .off
        }
    }

    /// "1 match", "2 matches": inflected here, because a status label takes
    /// a plain string and a plain string does not inflect itself.
    static func matchCount(_ count: Int) -> String {
        String(AttributedString(localized: "^[\(count) match](inflect: true)").characters)
    }

    /// The keystroke path. A local in-memory pass over the already-open
    /// redacted preview and nothing else.
    ///
    /// Runs on the main actor deliberately: the scan is a local in-memory
    /// pass, and keeping every touch of the `tc_preview*` on one thread is
    /// what the header's ownership rules ask for -- its pointer check narrows
    /// accidental misuse to an error, it does not make concurrent use safe.
    ///
    /// It deliberately does NOT ask about the original. `searchOriginal`
    /// bottoms out in `tc_search_original`, which spawns a thread, builds a
    /// runtime, and reads the whole raw unredacted session file off disk;
    /// on `.onChange(of: needle)` that ran once per character typed, on this
    /// actor. The outcome is cleared rather than left standing, so a verdict
    /// from the previous term is never shown against the new one.
    private func run() {
        searched = true
        outcome = nil
        guard !needle.isEmpty, let preview else {
            offsets = []
            return
        }
        offsets = preview.search(needle)
    }

    /// Runs the search, asks about the original, AND records the term.
    ///
    /// Separate from `run` for two reasons, and both are about the
    /// difference between passing through a prefix and asking a question.
    ///
    /// Remembering here because doing it in `run` filled the six-slot strip
    /// with the prefixes of one word: typing "xyz" recorded "x", "xy", and
    /// "xyz". Checking the original here because that check is the expensive
    /// one -- see `run` -- and a contributor asks it by pressing Return or
    /// the button.
    private func commit() {
        run()
        guard !needle.isEmpty, offsets != nil else { return }
        // The redacted-body count alone cannot tell "we took it out" from
        // "it was never here". See `OriginalSearchOutcome`.
        outcome = OriginalSearchOutcome.classify(
            remaining: offsets?.count ?? 0,
            original: searchOriginal(needle)
        )
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
            let snippet = document.snippet(around: offset, matchBytes: needle.utf8.count, window: 120)
            guard !snippet.text.isEmpty else { return "" }
            let text = snippet.text.replacingOccurrences(of: "\n", with: " ")
            return (snippet.elidedBefore ? "…" : "") + text + (snippet.elidedAfter ? "…" : "")
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
    /// Ron's Look-inside words: the caption, Load more and Add turn
    /// separators. Nil draws none of them.
    var words: MonitorLookInsideCopy? = nil
    /// The core's turn index over this body, drawn as separators. Empty
    /// until asked for.
    var turns: [PreviewTurns.Turn] = []
    /// How many chunks are shown (Ron's pages); the rest wait on Load more.
    var shownChunks: Int? = nil
    var onLoadMore: () -> Void = {}
    /// Asks for the turn index; nil when it cannot be asked yet or is in.
    var onAddSeparators: (() -> Void)? = nil
    /// The chunks that are typeset right now, and the eviction that keeps
    /// that set under the ceiling. The policy lives in `TCShellCore` so it
    /// can be asserted against real byte counts without a running app.
    @State private var resident = TranscriptResidentChunks<ChippedChunk>()
    /// Where each chunk sits vertically, so a chunk that is not typeset
    /// still holds its place in the scroll.
    @State private var rows: TranscriptRowIndex?
    /// The last chunk to come into view; the window is centred on it, so
    /// overscan follows the reader in whichever direction they are going.
    @State private var anchor = 0
    @State private var columns = 0
    @State private var copied = false

    /// The horizontal inset the chunks are drawn inside: a quiet card's own.
    /// `measure` takes it off both sides, so the column count is the one the
    /// text is actually given.
    static let inset = GlassTokens.Space.s6

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s5) {
            HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
                // Ron's caption, from the core.
                Text(words?.transcriptCaption ?? "")
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 0)
                Button(copied ? "Copied" : "Copy everything", action: copyAll)
                    .buttonStyle(GlassButtonStyle(.glass))
                    .help(
                        "Puts the whole redacted body on the clipboard. "
                            + "Selection inside the transcript covers one block at a time."
                    )
                    .accessibilityIdentifier("transcript-copy-all")
            }

            GeometryReader { geometry in
                // Flush, so the one inset is the stack's and `measure` can
                // take exactly it off the card's width.
                GlassCard(quiet: true, flush: true) {
                    CaptureSafeScroll {
                        LazyVStack(alignment: .leading, spacing: 0) {
                            ForEach(laidOutIndices, id: \.self) { index in
                                chunkRow(index)
                            }
                        }
                        .padding(.horizontal, TranscriptTab.inset)
                        .padding(.vertical, GlassTokens.Space.s5)
                    }
                }
                .onAppear { measure(width: geometry.size.width) }
                .onChange(of: geometry.size.width) { _, width in measure(width: width) }
            }

            // Ron's Load more, naming what is left, and his link to the
            // turn separators once the whole body is shown.
            if let words, remaining > 0 || onAddSeparators != nil {
                HStack(spacing: GlassTokens.Space.s4) {
                    if remaining > 0 {
                        Button(FirstRunCopy.fill(words.loadMore, ["size": Format.bytes(remaining)]), action: onLoadMore)
                            .buttonStyle(GlassButtonStyle(.glass))
                    }
                    if let onAddSeparators {
                        Button(words.addTurnSeparators, action: onAddSeparators)
                            .buttonStyle(GlassButtonStyle(.link))
                    }
                    Spacer(minLength: 0)
                }
            }
        }
    }

    /// The chunks on screen: all of them, or as many as the pages so far.
    private var shown: Int { min(document.chunkCount, max(0, shownChunks ?? document.chunkCount)) }

    /// The bytes still behind Load more.
    private var remaining: Int { LookInside.remainingBytes(document, shownChunks: shown) }

    /// A turn's separator, drawn where the turn opens in the body: a
    /// hairline and the core's index row for it.
    private func separator(_ turn: PreviewTurns.Turn) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
            Divider().overlay(GlassColor.hairline)
            HStack(spacing: GlassTokens.Space.s3) {
                Text(LookInside.turnTitle(turn))
                    .glassType(GlassTokens.TypeScale.label)
                    .foregroundStyle(GlassColor.textSecondary)
                if let words {
                    Text(LookInside.turnDetail(turn, words: words))
                        .glassType(GlassTokens.TypeScale.caption)
                        .monospacedDigit()
                        .foregroundStyle(GlassColor.textTertiary)
                }
            }
            .accessibilityElement(children: .combine)
        }
        .padding(.vertical, GlassTokens.Space.s2)
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
        guard CaptureMode.isRendering else { return 0..<shown }
        let window = TranscriptResidency.window(document, visible: 0..<1)
        return window.lowerBound..<min(window.upperBound, shown)
    }

    @ViewBuilder
    private func chunkRow(_ index: Int) -> some View {
        Group {
            if let chunk = resident.rendered[index] {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(chunk.segments.enumerated()), id: \.offset) { _, segment in
                        if let turn = segment.turn {
                            separator(turn)
                        }
                        Text(segment.text)
                            .glassType(GlassTokens.TypeScale.mono)
                            .textSelection(.enabled)
                            .foregroundStyle(GlassColor.textPrimary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            // The chips are named here and nowhere else:
                            // SwiftUI has no per-run accessibility label
                            // inside a `Text`, and a marker left unnamed is
                            // spelled out as punctuation and capitals in the
                            // middle of a sentence. See `RedactionMarks`.
                            .accessibilityLabel(segment.spoken)
                    }
                }
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
        let turns = self.turns
        resident.update(document: document, visible: index..<(index + 1)) { chunk in
            let text = document.text(of: chunk)
            let meta = document.chunks[chunk]
            let segments = LookInside.segments(of: meta, turns: turns)
            guard segments.count > 1 || segments.first?.turn != nil else {
                return ChippedChunk(segments: [ChippedSegment(
                    turn: nil,
                    text: TranscriptMarkers.chipped(text, font: GlassTokens.TypeScale.mono.font),
                    spoken: RedactionMarks.spoken(text)
                )])
            }
            // Cut at each turn's byte offset. The offsets fall between
            // events, never inside a character or a redaction marker.
            let bytes = Array(text.utf8)
            return ChippedChunk(segments: segments.map { segment in
                let lower = segment.byteRange.lowerBound - meta.byteOffset
                let upper = segment.byteRange.upperBound - meta.byteOffset
                let piece = String(decoding: bytes[lower..<upper], as: UTF8.self)
                return ChippedSegment(
                    turn: segment.turn,
                    text: TranscriptMarkers.chipped(piece, font: GlassTokens.TypeScale.mono.font),
                    spoken: RedactionMarks.spoken(piece)
                )
            })
        }
    }

    private func measure(width: CGFloat) {
        let usable = max(1, width - 2 * TranscriptTab.inset)
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
    /// The line the mono step sets: the font's own line plus the spacing
    /// `glassType` adds to reach the step's line height.
    private static let rowHeight =
        NSLayoutManager().defaultLineHeight(for: font) + GlassTokens.TypeScale.mono.lineSpacing
}

/// One resident chunk: what it draws as, and what it reads as aloud.
///
/// The spoken form is built in the same pass as the chips, off the same
/// text, so naming costs one scan per chunk that was going to be scanned
/// anyway -- and a chunk that is evicted drops both together rather than
/// leaving a name behind for text nobody is holding.
private struct ChippedChunk {
    /// The chunk, cut where a turn opens inside it; one piece when none does.
    let segments: [ChippedSegment]
}

/// One piece of a resident chunk, and the turn that opens it, if any.
private struct ChippedSegment {
    let turn: PreviewTurns.Turn?
    let text: AttributedString
    /// The piece with each marker replaced by its name. See `RedactionMarks`.
    let spoken: String
}

/// Turns the redaction pipeline's `<PRIVATE_*>` and `[REDACTED*]` markers
/// into chips: bold, primary text on the selected-control fill rather than
/// the ask colour, so they read as objects placed in the text instead of
/// damage done to it. TCDesign has no chip token and this adds none (ruling
/// R-13).
///
/// Runs per chunk now, never over the whole body. The scan itself is in
/// `TranscriptMarkerScan` and is shared with the chunker, which uses it to
/// avoid cutting through a marker -- half a marker rendered as body text in
/// one block and the other half in the next would read as content that was
/// never scrubbed.
///
/// The chip's colours are deliberate and are not the ask colour; that is
/// the paragraph above and it stands. What the chip does NOT carry is a name:
/// every one of them draws the same whether it stands for a path, a
/// credential, or a name found in prose. `RedactionMarks` supplies that,
/// over this same scan, and `chunkRow` puts it on the chunk's accessibility
/// label -- SwiftUI has no per-run label inside a `Text`, so the chunk is
/// the finest grain available.
private enum TranscriptMarkers {
    static func chipped(_ text: String, font: Font) -> AttributedString {
        var out = AttributedString()
        var cursor = text.startIndex
        for range in TranscriptMarkerScan.spans(in: text) {
            out.append(AttributedString(String(text[cursor..<range.lowerBound])))
            var chip = AttributedString(String(text[range]))
            chip.font = font.weight(.bold)
            chip.backgroundColor = GlassTokens.Color.controlSelected.color
            chip.foregroundColor = GlassColor.textPrimary
            out.append(chip)
            cursor = range.upperBound
        }
        out.append(AttributedString(String(text[cursor...])))
        return out
    }
}




/// The disclosure is scrollable and shared with the screenshot renderer.
///
/// The sheet is the confirmation: nothing is sent to the witness until
/// Confirm, and Cancel (or Escape) leaves the preview exactly as it was.
struct WitnessReviewConsent: View {
    let copy: WitnessReviewCopy
    /// Ron's confirmation tick (#1146 `WitnessReviewOverlay`), in the
    /// core's words: Confirm waits on it. Nil draws no tick and Confirm
    /// stays disabled: a caller without the core's words cannot confirm.
    let confirmLine: String?
    let confirmLabel: String?
    let onCancel: () -> Void
    let onConfirm: () -> Void
    @State private var confirmed = false

    init(copy: WitnessReviewCopy, confirmLine: String? = nil, confirmLabel: String? = nil,
         onCancel: @escaping () -> Void, onConfirm: @escaping () -> Void) {
        self.copy = copy
        self.confirmLine = confirmLine
        self.confirmLabel = confirmLabel
        self.onCancel = onCancel
        self.onConfirm = onConfirm
    }

    /// Confirm is never the default: Return does not start a review.
    var body: some View {
        GlassModal(
            title: copy.heading, width: .narrow,
            actions: [
                .cancel(copy.cancel, action: onCancel),
                GlassModalAction(copy.confirm, isEnabled: confirmLine != nil && confirmed, isProminent: true) {
                    onCancel()
                    onConfirm()
                },
            ],
            onCancel: onCancel
        ) {
            GlassModalBody { disclosure }
        }
    }

    private var disclosure: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s8) {
            Text(copy.disclosure)
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
            Text(copy.immutable)
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            if let confirmLine {
                GlassCheckRow(confirmLine, isOn: $confirmed)
                    .accessibilityLabel(confirmLabel ?? confirmLine)
            }
        }.frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// The preview's own surface: the pane tier in a stock sheet, none in a
/// modal, which is already the pane.
private struct PreviewChrome: ViewModifier {
    let inModal: Bool

    func body(content: Content) -> some View {
        if inModal {
            content
        } else {
            content.glassTier(.pane)
        }
    }
}

/// The preview raised in a `GlassModal` over the whole window: the same
/// tabs, gates and footer as the sheet, the modal titled with the sheet's
/// own Look-inside heading. Escape and Close both close it; nothing in it
/// answers Return.
struct PreviewModal: View {
    let entry: QueueEntry
    let onClose: () -> Void
    @EnvironmentObject private var model: AppModel

    var body: some View {
        GlassModal(
            title: PreviewSheet.modalTitle ?? model.publicRunCopy?.sessionDetail ?? PreviewSheet.reviewWord ?? "",
            onCancel: onClose
        ) {
            PreviewSheet(entry: entry, onClose: onClose)
        }
    }
}
