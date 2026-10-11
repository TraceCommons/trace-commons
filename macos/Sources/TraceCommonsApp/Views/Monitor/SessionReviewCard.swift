import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// What the review card holds for one session: the answer to the outcome
/// question and the correction written under Partly or Failed.
///
/// Keyed by the session it was written for, as `PreviewSlot` is: an
/// arrow-key step to another session never carries a verdict or a
/// correction over to it (`current(for:)`).
struct SessionReviewDraft: Equatable {
    let entryId: String
    private(set) var verdict: ContributorVerdict?
    private(set) var correction = ""

    init(entryId: String) {
        self.entryId = entryId
    }

    /// The correction field is drawn only under Partly or Failed: a run the
    /// contributor has just called successful has nothing to correct.
    var correctionOffered: Bool { verdict == .partly || verdict == .failed }

    /// What Contribute sends: the trimmed text under Partly or Failed, and
    /// nil otherwise or when blank, which sends no key at all.
    var correctionToSend: String? {
        correctionOffered ? CorrectionCopy.toSend(correction) : nil
    }

    /// One answer pressed. The chosen answer again takes it back. What was
    /// written under Partly or Failed is kept when the answer leaves them
    /// (the arrow keys step through Worked on the way from Partly to
    /// Failed), but it is sent only while they are chosen
    /// (`correctionToSend`), so text nobody can see never rides along on
    /// the approval.
    mutating func choose(_ option: ContributorVerdict) {
        verdict = verdict == option ? nil : option
    }

    /// The correction as typed, held to the core's limit at the keyboard so
    /// an over-long one is shortened where it can be seen.
    mutating func write(_ text: String, limit: Int) {
        correction = text.count > limit ? String(text.prefix(limit)) : text
    }

    /// This draft for `entryId`, or a fresh one for any other session.
    func current(for entryId: String) -> SessionReviewDraft {
        entryId == self.entryId ? self : SessionReviewDraft(entryId: entryId)
    }
}

/// The selected session's card, which is its review (Ron's `WaitingReview`,
/// `waiting-review.tsx`): what would leave this computer, the verdict and
/// the correction, and Contribute, the one approve control in the Monitor.
/// Native's Keep, and the queue card's facts, are kept inside it.
///
/// Every word is the core's: the Traces table (`sessionReview`), the
/// disclosure bundle's outcome table, the consent gate, and the shared
/// eligibility, redaction and residual-secret tables.
struct SessionReviewCard: View {
    let store: TracesStore
    let entry: DaemonData.QueueEntry?
    /// The tab's words, from the core, decoded once by the store.
    private var words: MonitorTracesCopy? { store.words }
    private var review: MonitorSessionReviewCopy? { store.words?.sessionReview }
    /// The verdict and correction words, from the core's disclosure bundle.
    private var outcome: ContributorDisclosureCopy.Outcome? { store.disclosure?.outcome }
    /// The legacy queue's entries, which the preview sheet still takes.
    @EnvironmentObject private var model: AppModel
    /// The session whose preview sheet is open, as the legacy queue holds it.
    @State private var previewing: QueueEntry?
    /// The verdict and correction, for the session they were written for.
    @State private var draft = SessionReviewDraft(entryId: "")
    /// Whether the trace's facts are shown; collapsed by default.
    @State private var detailsOpen = false

    /// The preview, keyed by the session it was asked for. Only the
    /// selected session's answer is ever read out of it.
    @State private var slot = PreviewSlot()
    private var summary: DaemonData.PreviewSummary? { slot.summary(for: entry?.entryId) }
    private var failure: DaemonDataError? { slot.failure(for: entry?.entryId) }

    /// How long a selection must rest before its preview is asked for. Each
    /// arrow-key step cancels the wait, so stepping through the tree starts
    /// no preview until it stops; `preview` is a full read-parse-redact pass
    /// the daemon cannot cancel.
    static let previewSettle: Duration = .milliseconds(300)
    /// The consent gate, from the Rust core, decoded once by the store.
    private var consent: ConsentCopy? { store.consent }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            if let entry {
                ScrollView {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                        // Ron's `InspectorHeader`: the session tile, the
                        // folder it ran in, and "Session · tool".
                        InspectorHeader(
                            tile: .session, title: entry.projectLabel, sub: words.map { Self.headerSub(entry, words: $0) })
                        if let review {
                            // Ron's review card ends with its buttons.
                            GlassCard(quiet: true) {
                                VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                                    preview(entry, review)
                                    actions(entry)
                                }
                                .frame(maxWidth: .infinity, alignment: .leading)
                            }
                        }
                        keptLines(entry)
                    }
                }
                .scrollIndicators(.never)
                .task(id: entry.entryId) { await load(entry.entryId) }
            } else {
                Spacer(minLength: 0)
            }
        }
        // Over the whole window (`glassModalHost` at its root), not a sheet.
        .glassModal(item: $previewing) { PreviewModal(entry: $0) { previewing = nil }.environmentObject(model) }
        .onChange(of: model.awaitingDecision.count) { _, _ in
            // Development hook, as the legacy queue's: opens the first
            // preview so the sheet can be captured. Never on by default.
            if ProcessInfo.processInfo.environment["TRACE_COMMONS_DEMO_PREVIEW"] == "1",
                previewing == nil,
                let first = model.awaitingDecision.first
            {
                previewing = first
            }
        }
    }

    // MARK: Ron's review, in his order

    /// Loading or the read that failed; then what would leave this
    /// computer, the redaction summary, the surviving secret, the residual
    /// risk, the consent scopes, eligibility, the
    /// verdict and correction. The credential refusal is said under the
    /// buttons (`actions`).
    @ViewBuilder
    private func preview(_ entry: DaemonData.QueueEntry, _ review: MonitorSessionReviewCopy) -> some View {
        if let failure {
            GlassNotice(tone: .outside, title: review.cannotShowTitle) {
                Text(words?.line(for: failure) ?? review.cannotShowBody)
            }
        } else if let summary {
            // The eyebrow and the enrolment chip share a line, so the
            // heading runs the card's width rather than wrapping beside the
            // chip (owner, 2026-10-10: a less crowded card).
            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                HStack(alignment: .center, spacing: GlassTokens.Space.s4) {
                    Text(review.eyebrow)
                        .glassType(GlassTokens.TypeScale.eyebrow)
                        .foregroundStyle(GlassColor.textTertiary)
                    Spacer(minLength: 0)
                    // Unknown is not enrolled: Contribute stays disarmed.
                    // #1146's glass chip, no dot; Not enrolled is tinted and
                    // secondary.
                    GlassChip(glass: summary.enrolled == true ? review.enrolled : review.notEnrolled,
                              muted: summary.enrolled != true)
                }
                Text(review.heading)
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .foregroundStyle(GlassColor.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            GlassCard(quiet: true) {
                Text(summary.openingPrompt.flatMap { $0.isEmpty ? nil : $0 } ?? review.noOpeningPrompt)
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textPrimary)
                    .lineLimit(8)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            if let measures = Self.measures(summary, review) {
                Text(measures)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
            }
            redactions(summary, review)
            if let survivor = TracesStore.survivorLine(summary) {
                GlassStatusLabel(survivor, status: .ask)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityLabel(survivor)
            } else if RedactionLabels.survivorTotal(summary.redactions ?? [:]) > 0 {
                caption(review.residualUnavailable, outside: true)
            }
            // #1146: one tertiary caption line each, the label in bold and
            // the core's values as it sends them.
            if let words, let risk = summary.residualRisk, !risk.isEmpty {
                labelled(words.residualRisk, risk)
            }
            // The gate statement is not repeated here (owner, 2026-10-10):
            // the scrubbing caveat over the buttons says the same, and the
            // Look inside sheet keeps the statement in full.
            if let scopes = summary.consentScopes, !scopes.isEmpty {
                labelled(review.consentScopes, scopes.joined(separator: " · "), bold: false)
            }
            eligibility(entry, review)
            if let outcome {
                verdict(outcome)
            } else {
                caption(review.outcomeUnavailable, outside: true)
            }
        } else {
            caption(review.buildingPreview)
        }
    }

    /// "{size} redacted payload" and "{count} events", each only when the
    /// daemon reported it.
    static func measures(_ summary: DaemonData.PreviewSummary, _ review: MonitorSessionReviewCopy) -> String? {
        let parts = [
            summary.wouldSendBytes.map {
                FirstRunCopy.fill(review.redactedPayload, [
                    "size": ByteCountFormatter.string(fromByteCount: Int64($0), countStyle: .memory),
                ])
            },
            summary.eventCount.map { FirstRunCopy.fill(review.events, ["count": String($0)]) },
        ].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    /// What scrubbing removed, and what it found but left in, as the core
    /// groups, splits and words it (`tc_redaction_summary_json`). No counts,
    /// or an answer that will not parse, says the summary is unavailable and
    /// disarms Contribute: an unread count is never "nothing matched".
    /// Distinct counts are sent only with a full summary; absent, the core
    /// reads them as none.
    static func redactionRows(
        _ summary: DaemonData.PreviewSummary
    ) -> (removed: [RedactionSummaryRow], stillPresent: [RedactionSummaryRow])? {
        guard let occurrences = summary.redactions else { return nil }
        return RedactionSummary.rows(fromJSON: TCCoreCopy.redactionSummaryJSON(
            occurrences: occurrences, distinct: summary.redactionsDistinct ?? [:]))
    }

    @ViewBuilder
    private func redactions(_ summary: DaemonData.PreviewSummary, _ review: MonitorSessionReviewCopy) -> some View {
        if let rows = Self.redactionRows(summary) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                Text(review.removed)
                    .glassType(GlassTokens.TypeScale.label)
                    .foregroundStyle(GlassColor.textPrimary)
                if rows.removed.isEmpty {
                    caption(review.nothingRemoved)
                } else {
                    // The count and what it covered; what the category is
                    // goes behind the line's tooltip and accessibility hint
                    // (owner, 2026-10-10: a less verbose card). What was
                    // left in keeps its full line below.
                    ForEach(rows.removed, id: \.family) { row in
                        Self.redactionLine(row, describe: false)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .help(row.description)
                            .accessibilityHint(row.description)
                    }
                }
            }
            if !rows.stillPresent.isEmpty {
                GlassNotice(tone: .outside, title: review.stillPresent) {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                        ForEach(rows.stillPresent, id: \.family) { row in
                            Self.redactionLine(row)
                        }
                    }
                }
            }
        } else {
            caption(review.redactionsUnavailable, outside: true)
        }
    }

    /// Whether the session may go, in the shared table's sentences: its
    /// state and its reason. Words that will not decode say eligibility
    /// could not be checked, and Contribute stays disarmed.
    @ViewBuilder
    private func eligibility(_ entry: DaemonData.QueueEntry, _ review: MonitorSessionReviewCopy) -> some View {
        let eligibility = TracesStore.eligibility(entry)
        if eligibility != nil {
            if let copy = store.inferenceCopy,
                let state = EligibilitySurface.stateLine(eligibility, copy: copy, calls: TracesStore.eligibilityCalls)
            {
                caption(state)
                if let reason = EligibilitySurface.reasonLine(eligibility, calls: TracesStore.eligibilityCalls) {
                    caption(reason)
                }
            } else {
                caption(review.eligibilityFailed, outside: true)
            }
        }
    }

    /// The outcome question, its three answers, and under Partly or Failed
    /// the correction field with its disclosure and its count.
    @ViewBuilder
    private func verdict(_ outcome: ContributorDisclosureCopy.Outcome) -> some View {
        let draft = self.draft.current(for: entry?.entryId ?? "")
        let busy = entry.map { store.acting.contains($0.entryId) } ?? true
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            // #1146's legend: 12/600, primary.
            Text(outcome.verdictQuestion)
                .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                .foregroundStyle(GlassColor.textPrimary)
            // Radios, one answer at most (owner, 2026-10-10); pressing the
            // chosen one again clears it, as the answer is optional.
            GlassRadioRow(
                outcome.verdictQuestion, selection: draft.verdict,
                options: [
                    GlassRadioOption(outcome.worked, value: ContributorVerdict.worked),
                    GlassRadioOption(outcome.partly, value: ContributorVerdict.partly),
                    GlassRadioOption(outcome.failed, value: ContributorVerdict.failed),
                ]
            ) { option in
                var next = draft
                next.choose(option)
                self.draft = next
            }
            caption(outcome.verdictCaption)
            if draft.correctionOffered {
                Text(outcome.correctionQuestion)
                    .glassType(GlassTokens.TypeScale.label)
                    .foregroundStyle(GlassColor.textPrimary)
                // The question above names it; the cap works on the binding.
                GlassTextArea(outcome.correctionQuestion, text: Binding(
                    get: { draft.correction },
                    set: { text in
                        var next = self.draft.current(for: draft.entryId)
                        next.write(text, limit: outcome.maxCorrectionChars)
                        self.draft = next
                    }), showsLabel: false)
                    .frame(maxHeight: 140)
                    .accessibilityHint(outcome.correctionPlaceholder)
                // The disclosure that a correction is stored as written:
                // printed in full, never shortened for layout.
                caption(outcome.correctionCaption, tertiary: true)
                Text("\(draft.correction.count)/\(outcome.maxCorrectionChars)")
                    .glassType(GlassTokens.TypeScale.micro)
                    .foregroundStyle(GlassColor.textSecondary)
                    .frame(maxWidth: .infinity, alignment: .trailing)
            }
        }
        .disabled(busy)
    }

    // MARK: Native's kept lines

    /// What the queue card said that Ron's review does not: what scrubbing
    /// did and what that does not prove, whether delegated subagent
    /// transcripts were trimmed to fit, the second-look reasons, and the
    /// session's facts ("What's in it"). The surviving secret is said once,
    /// in the review above.
    @ViewBuilder
    private func keptLines(_ entry: DaemonData.QueueEntry) -> some View {
        // Only once the preview has counted: an unread count is never
        // read as "nothing matched".
        if let redactions = summary?.redactions {
            let removed = RedactionLabels.removedTotal(redactions)
            GlassStatusLabel(
                ScrubbingCaveat.rowLine(redactionCount: removed),
                status: ScrubbingCaveat.status(redactionCount: removed))
                .fixedSize(horizontal: false, vertical: true)
        }
        // A load-time fact on the entry, so it is said before the preview
        // is in: a trimmed conversation never reaches a decision unsaid.
        if let line = SubagentCopy.line(count: entry.subagentCount ?? 0, dropped: entry.subagentsDropped ?? 0) {
            if (entry.subagentsDropped ?? 0) > 0 {
                GlassStatusLabel(line, status: .ask)
                    .fixedSize(horizontal: false, vertical: true)
            } else {
                caption(line)
            }
        }
        if let reasons = entry.secondLook, !reasons.isEmpty {
            // The core's fixed reason labels until K4 gives them words.
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                ForEach(reasons, id: \.self) { GlassChip($0, status: .ask) }
            }
        }
        // The session's facts and the token distribution, collapsed under
        // one heading (owner, 2026-10-10: a less crowded inspector). They
        // describe the trace; nothing in them is needed before deciding.
        if let words, let review {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                GlassExpander(review.traceDetails, isOpen: $detailsOpen)
                if detailsOpen {
                    GlassKeyValueList(Self.rows(
                        entry, summary, words: words, attestation: store.attestationValue(entry)))
                    // The daemon's own line, which the deleted What's-in-it
                    // tab drew. Absent draws nothing: no fact is invented.
                    if let line = summary?.tokenDistributionSummary, !line.isEmpty {
                        caption(line)
                    }
                }
            }
        }
    }

    // MARK: Actions

    /// Contribute, Dismiss and Look inside on one line, left-aligned: the
    /// action first as the primary button, then Dismiss and Look inside as
    /// links, the order the owner ruled for every card (Ron, 2026-10-09).
    /// No label wraps or shortens: a Contribute label too long for the line
    /// ("Enroll to approve") moves Look inside onto a line of its own
    /// below, rather than clip the one button that sends. Native's Keep is
    /// a link on the line under them. The scrubbing caveat sits directly
    /// above the buttons, at reading weight, as the review sheet repeats it
    /// at the commit; a refused action is said directly below them.
    @ViewBuilder
    private func actions(_ entry: DaemonData.QueueEntry) -> some View {
        let busy = store.acting.contains(entry.entryId)
        ScrubbingCaveatAtCommit()
        if let words, let review {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: GlassTokens.Space.s4) {
                        contribute(entry, words)
                        dismiss(entry, words)
                        lookInside(entry, review)
                    }
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                        HStack(spacing: GlassTokens.Space.s4) {
                            contribute(entry, words)
                            dismiss(entry, words)
                        }
                        lookInside(entry, review)
                    }
                }
                Button(words.keep) { act(.keep, entry) }
                    .buttonStyle(GlassButtonStyle(.link))
                    .lineLimit(1)
                    .fixedSize()
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .disabled(busy)
        }
        // A refused action, then a refused correction, under the buttons
        // they are about (Ron, 2026-10-09).
        TracesRefusal(store: store, entryId: entry.entryId)
        if let outcome {
            if store.correctionRefused == entry.entryId {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                    GlassAlert(outcome.correctionCredentialHeadline)
                    GlassAlert(outcome.correctionCredentialBody)
                }
            }
        }
    }

    /// The full read of this same session, always offered: the legacy
    /// queue's entry when it holds one, otherwise the same entry carried
    /// over (`QueueEntryBridge`).
    private func lookInside(_ entry: DaemonData.QueueEntry, _ review: MonitorSessionReviewCopy) -> some View {
        Button(review.lookInside) {
            previewing = QueueEntryBridge.previewEntry(entry, in: model.awaitingDecision)
        }
        .buttonStyle(GlassButtonStyle(.link))
        .lineLimit(1)
        .fixedSize()
    }

    private func dismiss(_ entry: DaemonData.QueueEntry, _ words: MonitorTracesCopy) -> some View {
        Button(words.dismissAction) { act(.dismiss, entry) }
            .buttonStyle(GlassButtonStyle(.link))
            .lineLimit(1)
            .fixedSize()
    }

    private func contribute(_ entry: DaemonData.QueueEntry, _ words: MonitorTracesCopy) -> some View {
        Button(Self.contributeLabel(
            enrolled: summary?.enrolled, eligibility: TracesStore.eligibility(entry),
            eligibilityReadable: store.inferenceCopy != nil, words: words)
        ) { act(.contribute, entry) }
            .buttonStyle(GlassButtonStyle(.primary, small: true))
            .lineLimit(1)
            .fixedSize()
            .disabled(!armed(entry))
            // Why it is armed or not, in the core's words, as the review
            // sheet's Contribute says it; over a core that stopped
            // answering, that it stopped, never the consent gate's reason.
            .help(Self.coreAnswering(store.phase)
                ? (consent == nil ? "" : TCConsentCopy.gateHelp(pinned: armed(entry)) ?? "")
                : Self.coreDownHelp)
    }

    /// "Session · <tool>" in the core's words, the tool the session
    /// declares before the adapter that stored it.
    static func headerSub(_ entry: DaemonData.QueueEntry, words: MonitorTracesCopy) -> String {
        let tool = SourceKind(rawValue: entry.declaredSource ?? entry.source)?.displayName ?? entry.source
        return FirstRunCopy.fill(words.inspector.sessionOf, ["tool": tool])
    }

    /// Contribute's label, Ron's order: not enrolled, then not eligible,
    /// then eligibility whose words could not be read, then Contribute.
    static func contributeLabel(
        enrolled: Bool?, eligibility: ContributionEligibility?, eligibilityReadable: Bool, words: MonitorTracesCopy
    ) -> String {
        if enrolled == false { return words.sessionReview.enrollToApprove }
        if eligibility != nil {
            if !eligibilityReadable { return words.sessionReview.checkingEligibility }
            if !EligibilitySurface.offersContribute(eligibility, calls: TracesStore.eligibilityCalls) {
                return words.sessionReview.notEligible
            }
        }
        return words.contribute
    }

    /// Contribute's gate for `entry`, on the preview asked for it: a core
    /// that answers, the store's gate, and every word the review must show
    /// in front of it -- the outcome table, the redaction summary, and a
    /// surviving secret's line when one survived.
    private func armed(_ entry: DaemonData.QueueEntry) -> Bool {
        let summary = slot.summary(for: entry.entryId)
        return Self.coreAnswering(store.phase) && TracesStore.contributeArmed(
            enrolled: summary?.enrolled, consent: consent,
            eligibility: TracesStore.eligibility(entry), calls: TracesStore.eligibilityCalls)
            && Self.reviewShown(summary, outcome: outcome)
    }

    /// Whether the core answered the Traces store's last read. A card left
    /// up over a core that stopped answering (Home and History keep the
    /// Traces selection's card) is not armed: its banner says why.
    static func coreAnswering(_ phase: TracesStore.Phase) -> Bool {
        if case .failed = phase { return false }
        return true
    }

    /// Contribute's tooltip while the core is not answering: the core-down
    /// banner's own title, else the core's unknown word. Never the consent
    /// gate's not-ready reason, which would blame a missing enrollment for
    /// a core that is down.
    static var coreDownHelp: String {
        TracesHealth.coreDownLine?.title ?? TracesHealth.unknownWord ?? ""
    }

    /// Whether everything the card must say before Contribute could be
    /// said for `summary`.
    static func reviewShown(_ summary: DaemonData.PreviewSummary?, outcome: ContributorDisclosureCopy.Outcome?) -> Bool {
        guard outcome != nil, let summary, redactionRows(summary) != nil else { return false }
        let survived = RedactionLabels.survivorTotal(summary.redactions ?? [:]) > 0
        return !survived || TracesStore.survivorLine(summary) != nil
    }

    private func act(_ action: TracesStore.ReviewAction, _ entry: DaemonData.QueueEntry) {
        if action == .contribute {
            // Asked again at the press, on the session as the tree now has
            // it: the summary or the eligibility may have moved since the
            // button was drawn. The draft is this session's, never another's.
            guard let live = store.tree.allSessions.first(where: { $0.entryId == entry.entryId }),
                armed(live)
            else { return }
            let draft = self.draft.current(for: entry.entryId)
            Task {
                await store.perform(.contribute, on: entry.entryId, verdict: draft.verdict, correction: draft.correctionToSend)
            }
            return
        }
        Task { await store.perform(action, on: entry.entryId) }
    }

    private func load(_ entryId: String) async {
        slot.begin(entryId)
        draft = draft.current(for: entryId)
        do {
            try await Task.sleep(for: Self.previewSettle)
        } catch {
            return
        }
        let result: Result<DaemonData.PreviewSummary, DaemonDataError>
        do {
            result = .success(try await store.attached().preview(entryId: entryId))
        } catch {
            result = .failure(error as? DaemonDataError ?? .undecodable(method: "preview"))
        }
        // A late answer for a session no longer selected is dropped.
        guard !Task.isCancelled else { return }
        slot.accept(entryId, result)
    }

    /// #1146's redaction line: the count in bold, then what the category
    /// is (unless `describe` is false) and the sub-labels it covered, all
    /// the core's words.
    static func redactionLine(_ row: RedactionSummaryRow, describe: Bool = true) -> Text {
        var tail = !describe || row.description.isEmpty ? "" : ": " + row.description
        if !row.detail.isEmpty { tail += " (" + row.detail.joined(separator: ", ") + ")" }
        return Text(row.countLine).bold().foregroundColor(GlassColor.textPrimary) + Text(tail)
    }

    /// "Label: value" as one tertiary caption line (#1146), the label bold
    /// where #1146 bolds it.
    private func labelled(_ label: String, _ value: String, bold: Bool = true) -> some View {
        (Text(label + ":").bold(bold) + Text(" " + value))
            .glassType(GlassTokens.TypeScale.caption)
            .foregroundStyle(GlassColor.textTertiary)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func caption(_ text: String, outside: Bool = false, tertiary: Bool = false) -> some View {
        Text(text)
            .glassType(GlassTokens.TypeScale.caption)
            .foregroundStyle(outside ? GlassTokens.Color.statusOutsideText.color : tertiary ? GlassColor.textTertiary : GlassColor.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// The session's facts, one word a row, and only what the daemon
    /// reported. Unknown is a dash. Eligibility is said in the review above
    /// and residual risk at Ron's place in it, so the card passes neither.
    static func rows(
        _ entry: DaemonData.QueueEntry, _ summary: DaemonData.PreviewSummary?, words: MonitorTracesCopy,
        eligibility: String? = nil, attestation: String? = nil
    ) -> [GlassKeyValueList.Item] {
        let dash = "—"
        func bytes(_ value: Int?) -> String {
            value.map { ByteCountFormatter.string(fromByteCount: Int64($0), countStyle: .memory) } ?? dash
        }
        func number(_ value: Int?) -> String { value.map(String.init) ?? dash }
        let tool = SourceKind(rawValue: entry.declaredSource ?? entry.source)?.displayName ?? entry.source
        var rows: [GlassKeyValueList.Item] = [
            .init(words.tool, tool),
            .init(words.folder, entry.projectLabel),
            .init(words.started, entry.startedAt.map { $0.formatted(date: .abbreviated, time: .shortened) } ?? dash),
            .init(words.length, entry.durationSecs.map {
                Duration.seconds($0).formatted(.units(allowed: [.hours, .minutes], width: .abbreviated))
            } ?? dash),
            .init(words.prompts, number(entry.userTurns)),
            .init(words.size, bytes(entry.sizeBytes)),
            .init(words.sends, bytes(summary?.wouldSendBytes)),
            // Absent until scrubbed: a dash, never zero.
            .init(words.marks, number(entry.marks)),
            .init(words.unsure, number(entry.unsureSpans)),
        ]
        // Whether it may go, and what the witness attested, in the core's
        // sentences; left out when the core has nothing to say.
        if let eligibility { rows.append(.init(words.eligibility, eligibility)) }
        if let attestation { rows.append(.init(words.attestation, attestation)) }
        // Only the full preview carries these. Categories only: the matched
        // text is never reported.
        if let labels = summary?.piiLabelsPresent, !labels.isEmpty {
            rows.append(.init(words.personalInformation, labels.joined(separator: ", ")))
        }
        return rows
    }
}

extension QueueEntryBridge {
    /// The entry Look inside opens for `entry`: the legacy queue's own when
    /// it holds the same session, otherwise the same session carried over
    /// field for field. Never another session's: the id is the entry's.
    /// A field the daemon did not report is the legacy entry's empty value,
    /// which the preview sheet only displays.
    static func previewEntry(_ entry: DaemonData.QueueEntry, in awaiting: [QueueEntry]) -> QueueEntry {
        if let legacy = legacyEntry(for: entry.entryId, in: awaiting) { return legacy }
        return QueueEntry(
            entryID: entry.entryId, sessionHash: entry.sessionHash ?? "", source: entry.source,
            declaredSource: entry.declaredSource, projectID: entry.projectId, projectLabel: entry.projectLabel,
            projectPath: entry.projectPath ?? "", sessionPath: entry.sessionPath, sizeBytes: entry.sizeBytes,
            discoveredAt: entry.discoveredAt ?? entry.startedAt,
            state: QueueState(rawValue: entry.state) ?? .pending, reasonLabel: entry.reasonLabel,
            attempts: entry.attempts ?? 0, subagentCount: entry.subagentCount,
            subagentsDropped: entry.subagentsDropped, eligibility: entry.eligibility,
            eligibilityReason: entry.eligibilityReason, attestation: entry.attestation,
            attestationReason: entry.attestationReason, holdsCertificateRaw: entry.holdsCertificate)
    }
}
