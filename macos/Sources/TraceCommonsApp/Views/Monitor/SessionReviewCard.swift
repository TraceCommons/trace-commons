#if DEBUG
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

    /// One answer pressed. The chosen answer again takes it back; leaving
    /// Partly or Failed withdraws what was written under them, so text
    /// nobody can see any more never rides along on the approval.
    mutating func choose(_ option: ContributorVerdict) {
        verdict = verdict == option ? nil : option
        if !correctionOffered { correction = "" }
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
                        Text(summary?.title ?? TracesTreeView.when(entry))
                            .glassType(GlassTokens.TypeScale.title)
                            .foregroundStyle(GlassColor.textPrimary)
                            .lineLimit(3)
                        if let review {
                            GlassCard(quiet: true) {
                                VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                                    preview(entry, review)
                                }
                                .frame(maxWidth: .infinity, alignment: .leading)
                            }
                        }
                        keptLines(entry)
                        actions(entry)
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
    /// risk, the gate statement, the consent scopes, eligibility, the
    /// verdict and correction, and the credential refusal.
    @ViewBuilder
    private func preview(_ entry: DaemonData.QueueEntry, _ review: MonitorSessionReviewCopy) -> some View {
        if let failure {
            GlassNotice(tone: .outside, title: review.cannotShowTitle) {
                Text(words?.line(for: failure) ?? review.cannotShowBody)
            }
        } else if let summary {
            HStack(alignment: .top, spacing: GlassTokens.Space.s4) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                    Text(review.eyebrow)
                        .glassType(GlassTokens.TypeScale.eyebrow)
                        .foregroundStyle(GlassColor.textTertiary)
                    Text(review.heading)
                        .glassType(GlassTokens.TypeScale.bodyStrong)
                        .foregroundStyle(GlassColor.textPrimary)
                }
                Spacer(minLength: 0)
                // Unknown is not enrolled: Contribute stays disarmed.
                GlassChip(summary.enrolled == true ? review.enrolled : review.notEnrolled,
                          status: summary.enrolled == true ? .on : .ask)
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
            if let words, let risk = summary.residualRisk, !risk.isEmpty {
                GlassKeyValueList([.init(words.residualRisk, risk.replacingOccurrences(of: "_", with: " "))])
            }
            if let consent {
                caption(consent.gateStatement)
            }
            if let scopes = summary.consentScopes, !scopes.isEmpty {
                GlassKeyValueList([.init(
                    review.consentScopes,
                    scopes.map { $0.replacingOccurrences(of: "_", with: " ") }.joined(separator: " · "))])
            }
            eligibility(entry, review)
            if let outcome {
                verdict(outcome)
                if store.correctionRefused == entry.entryId {
                    GlassNotice(tone: .outside, title: outcome.correctionCredentialHeadline) {
                        Text(outcome.correctionCredentialBody)
                    }
                }
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
                    "size": ByteCountFormatter.string(fromByteCount: Int64($0), countStyle: .file),
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
                    ForEach(rows.removed, id: \.family) { row in
                        caption(row.countLine)
                    }
                }
            }
            if !rows.stillPresent.isEmpty {
                GlassNotice(tone: .outside, title: review.stillPresent) {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                        ForEach(rows.stillPresent, id: \.family) { row in
                            Text(row.countLine)
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
            Text(outcome.verdictQuestion)
                .glassType(GlassTokens.TypeScale.label)
                .foregroundStyle(GlassColor.textPrimary)
            HStack(spacing: GlassTokens.Space.s2) {
                verdictOption(.worked, outcome.worked, draft)
                verdictOption(.partly, outcome.partly, draft)
                verdictOption(.failed, outcome.failed, draft)
                Spacer(minLength: 0)
            }
            .accessibilityElement(children: .contain)
            .accessibilityLabel(outcome.verdictQuestion)
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

    /// One answer, a glass pill, as the review sheet draws it. The chosen
    /// one leads with a checkmark, is drawn strong and carries the selected
    /// trait, so which is chosen is never the fill alone.
    private func verdictOption(
        _ option: ContributorVerdict, _ label: String, _ draft: SessionReviewDraft
    ) -> some View {
        let selected = draft.verdict == option
        return Button {
            var next = draft
            next.choose(option)
            self.draft = next
        } label: {
            HStack(spacing: GlassTokens.Space.s2) {
                if selected {
                    Image(systemName: "checkmark")
                        .imageScale(.small)
                        .accessibilityHidden(true)
                }
                Text(label)
            }
            .glassType(GlassTokens.TypeScale.label.weight(selected ? .semibold : .regular))
            .foregroundStyle(selected ? GlassColor.textPrimary : GlassColor.textSecondary)
            .padding(.horizontal, GlassTokens.Space.s6)
            .frame(minHeight: GlassTokens.Size.control)
            .glassTier(selected ? .controlSelected : .control)
        }
        .buttonStyle(GlassPressStyle())
        .accessibilityAddTraits(selected ? [.isSelected, .isButton] : .isButton)
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
        if let words {
            GlassKeyValueList(Self.rows(
                entry, summary, words: words, attestation: store.attestationValue(entry)))
        }
        // The token distribution, the daemon's own line, which the deleted
        // What's-in-it tab drew. Absent draws nothing: no fact is invented.
        if let line = summary?.tokenDistributionSummary, !line.isEmpty {
            caption(line)
        }
    }

    // MARK: Actions

    /// Look inside, Dismiss, Keep and Contribute. The scrubbing caveat sits
    /// directly above them, at reading weight, as the review sheet repeats
    /// it at the commit.
    @ViewBuilder
    private func actions(_ entry: DaemonData.QueueEntry) -> some View {
        let busy = store.acting.contains(entry.entryId)
        ScrubbingCaveatAtCommit()
        TracesRefusal(store: store, entryId: entry.entryId)
        if let words, let review {
            HStack(spacing: GlassTokens.Space.s4) {
                // The full read, on the legacy queue's entry for this same
                // session. Absent, not disabled, when the legacy queue does
                // not hold it: no sheet for another session.
                if let legacy = QueueEntryBridge.legacyEntry(for: entry.entryId, in: model.awaitingDecision) {
                    Button(review.lookInside) { previewing = legacy }
                        .buttonStyle(GlassButtonStyle(.glass))
                }
                Button(words.dismissAction) { act(.dismiss, entry) }
                    .buttonStyle(GlassButtonStyle(.glass))
                Button(words.keep) { act(.keep, entry) }
                    .buttonStyle(GlassButtonStyle(.glass))
                Spacer(minLength: 0)
                Button(Self.contributeLabel(
                    enrolled: summary?.enrolled, eligibility: TracesStore.eligibility(entry),
                    eligibilityReadable: store.inferenceCopy != nil, words: words)
                ) { act(.contribute, entry) }
                    .buttonStyle(GlassButtonStyle(.primary, small: true))
                    .disabled(!armed(entry))
                    // Why it is armed or not, in the core's words, as the
                    // review sheet's Contribute says it.
                    .help(consent == nil ? "" : TCConsentCopy.gateHelp(pinned: armed(entry)) ?? "")
            }
            .disabled(busy)
        }
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

    /// Contribute's gate for `entry`, on the preview asked for it: the
    /// store's gate, and every word the review must show in front of it --
    /// the outcome table, the redaction summary, and a surviving secret's
    /// line when one survived.
    private func armed(_ entry: DaemonData.QueueEntry) -> Bool {
        let summary = slot.summary(for: entry.entryId)
        return TracesStore.contributeArmed(
            enrolled: summary?.enrolled, consent: consent,
            eligibility: TracesStore.eligibility(entry), calls: TracesStore.eligibilityCalls)
            && Self.reviewShown(summary, outcome: outcome)
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
            value.map { ByteCountFormatter.string(fromByteCount: Int64($0), countStyle: .file) } ?? dash
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
#endif
