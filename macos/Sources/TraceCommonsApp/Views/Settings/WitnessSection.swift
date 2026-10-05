import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// Whether a sealed machine redacts a session before it is sent, and what
/// happened to the last one. Every string comes from the core's witness copy
/// and its two sentence calls; the card draws nothing of its own if the
/// payload did not arrive.
///
/// Witness trust is not a switch: a pinned witness certifies every
/// submission, a configured-but-unpinned one refuses every submission before
/// any network call. The card renders the state case, so local redaction and
/// a total upload outage cannot look alike.
struct WitnessSection: View {
    @EnvironmentObject private var model: AppModel
    // The three fields are the model's `witnessDraft`, so a refresh landing
    // mid-edit cannot replace a half-typed address and a section switch
    // cannot drop one (G8 of #1229). `nil` means nothing has been edited.
    @State private var showingInferenceDisclosure = false
    @State private var showingTokenDisclosure = false
    @State private var showingTokenCapture = false
    @State private var showingTokenDiscard = false

    var body: some View {
        // The container is always present, so `.onAppear` runs even when the
        // core's copy is missing and the card draws nothing.
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            if let copy = model.witnessCopy {
                card(copy)
            } else {
                // The unavailable branch; the refresh below follows this
                // container's closing brace directly.
                Color.clear.frame(width: 0, height: 0).accessibilityHidden(true)
            }
        }
        .onAppear {
            // Asked every time the card appears: the config is a file, and
            // the CLI writes to it too.
            model.refreshWitness()
        }
    }

    private func card(_ copy: WitnessCopy) -> some View {
        let state = model.witnessState
        return VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            GlassEyebrowCard(copy.heading) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    // What the witness is doing comes first. Before it has
                    // been read, the card says it is waiting rather than
                    // drawing its prose as if there were nothing to say.
                    if model.witnessRead != .answered {
                        SettingsReadNotice(model.witnessRead, retry: model.refreshWitness)
                    } else if let code = model.witnessStateCode {
                        stateBlock(code)
                    }
                    prose(copy.intro)
                    prose(copy.certificateMeans)

                    // What the last submission did, in the core's sentence.
                    if let line = WitnessSurface.lastResultLine(calls: model.witnessCalls) {
                        GlassStatusLabel(
                            line,
                            status: Self.tone(WitnessSurface.lastResultTone(calls: model.witnessCalls)))
                            .accessibilityElement(children: .combine)
                    }

                    if let state, WitnessSurface.offersConfigure(state) {
                        fields(copy)
                    }

                    // The way out, on every refusing state including one
                    // this build cannot name.
                    if let state, WitnessSurface.offersClear(state) {
                        Button(copy.clear) { model.clearWitness() }
                            .buttonStyle(GlassButtonStyle(.link))
                            .disabled(model.witnessBusy)
                        note(copy.clearNote)
                    }
                }
            }
            inferenceEvidence(copy)
            tokenContribution(copy)
            note(copy.appliesAtOnce)
        }
    }

    private func stateBlock(_ code: Int32) -> some View {
        let stateTone = Self.tone(WitnessSurface.tone(forState: code, calls: model.witnessCalls))
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            // A state this build cannot name has no sentence; none is made up.
            if let line = WitnessSurface.stateLine(code, calls: model.witnessCalls) {
                GlassStatusLabel(line, status: stateTone)
                    .accessibilityElement(children: .combine)
            }
            // The ABI's fixed operator label, verbatim.
            if let label = model.witnessStatus?.refusal ?? model.witnessLabel {
                GlassTag(label, tone: Self.tagTone(stateTone))
            }
        }
    }

    private func fields(_ copy: WitnessCopy) -> some View {
        let form = model.witnessDraft ?? WitnessForm.fromStatus(model.witnessStatus)
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            GlassTextField(copy.urlTitle, text: Binding(
                get: { form.url },
                set: { value in
                    var next = form
                    next.url = value
                    model.witnessDraft = next
                }
            ))
            .accessibilityLabel(copy.urlTitle)

            GlassTextField(copy.signingAddressTitle, text: Binding(
                get: { form.signingAddress },
                set: { value in
                    var next = form
                    next.signingAddress = value
                    model.witnessDraft = next
                }
            ))
            .accessibilityLabel(copy.signingAddressTitle)

            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                Text(copy.measurementsTitle)
                    .glassType(GlassTokens.TypeScale.eyebrow)
                    .foregroundStyle(GlassColor.textTertiary)
                // The count as the core's sentence; nil where there is no
                // witness to count for, and then nothing is drawn.
                if let line = model.witnessStatus?.pinnedMeasurementLine {
                    note(line)
                }
                // One measurement set per line. Emptying the box and saving
                // is refused by the ABI; there is no keep-what-is-there mode.
                GlassWell {
                    TextEditor(text: Binding(
                        get: { form.measurements },
                        set: { value in
                            var next = form
                            next.measurements = value
                            model.witnessDraft = next
                        }
                    ))
                    .glassType(GlassTokens.TypeScale.mono)
                    .foregroundStyle(GlassColor.textPrimary)
                    .scrollContentBackground(.hidden)
                    .frame(minHeight: 64)
                    .padding(GlassTokens.Space.s2)
                    .accessibilityLabel(copy.measurementsTitle)
                }
                note(copy.measurementsNote)
            }

            // Disabled until there is something pinnable to write.
            Button(copy.configure) {
                model.configureWitness(form)
                model.witnessDraft = nil
            }
            .buttonStyle(GlassButtonStyle(.glass))
            .disabled(!form.canConfigure || model.witnessBusy)
        }
    }

    private func inferenceEvidence(_ copy: WitnessCopy) -> some View {
        GlassEyebrowCard(copy.inferenceHeading) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                prose(copy.inferenceDisclosure)
                prose(copy.inferenceCaptureNote)
                prose(copy.inferenceScopeNote)
                // Absent is absent: no sentence and a disabled Enable.
                if model.daemonSettings == nil {
                    SettingsReadNotice(model.settingsRead, retry: model.refreshSettings)
                } else if let enabled = model.daemonSettings?.ironwireAttestedBodies {
                    GlassStatusLabel(
                        enabled ? copy.inferenceEnabled : copy.inferenceDisabled,
                        status: enabled ? .on : .off)
                        .accessibilityElement(children: .combine)
                }
                HStack(spacing: GlassTokens.Space.s3) {
                    Button(copy.inferenceEnable) { showingInferenceDisclosure = true }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .disabled(model.inferenceEvidenceBusy || model.daemonSettings?.ironwireAttestedBodies == nil)
                    Button(copy.inferenceDisable) {
                        Task { await model.setInferenceEvidence(false) }
                    }
                    .buttonStyle(GlassButtonStyle(.glass))
                    .disabled(model.inferenceEvidenceBusy || model.daemonSettings?.ironwireAttestedBodies == nil)
                }
                if model.inferenceEvidenceSaveFailed {
                    GlassNotice(tone: .outside) { Text(copy.inferenceSaveFailed) }
                }
            }
        }
        .confirmationDialog(copy.inferenceHeading, isPresented: $showingInferenceDisclosure, titleVisibility: .visible) {
            Button(copy.inferenceConfirm) {
                Task { await model.setInferenceEvidence(true, disclosureConfirmed: true) }
            }
            Button(copy.inferenceCancel, role: .cancel) { }
        } message: {
            Text([copy.inferenceDisclosure, copy.inferenceCaptureNote, copy.inferenceScopeNote].joined(separator: "\n\n"))
        }
    }

    private func tokenContribution(_ copy: WitnessCopy) -> some View {
        GlassEyebrowCard(copy.tokenHeading ?? "") {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                prose(copy.tokenDisclosure ?? "")
                prose(copy.tokenCaptureNote ?? "")
                prose(copy.tokenScopeNote ?? "")
                if model.daemonSettings == nil {
                    SettingsReadNotice(model.settingsRead, retry: model.refreshSettings)
                } else if let enabled = model.daemonSettings?.tokenDistributionsContribution {
                    GlassStatusLabel(
                        enabled ? (copy.tokenEnabled ?? "") : (copy.tokenDisabled ?? ""),
                        status: enabled ? .on : .off)
                        .accessibilityElement(children: .combine)
                }
                HStack(spacing: GlassTokens.Space.s3) {
                    Button(copy.tokenEnable ?? "") { showingTokenDisclosure = true }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .disabled(model.tokenContributionBusy || model.daemonSettings?.tokenDistributionsContribution == nil)
                    Button(copy.tokenDisable ?? "") {
                        Task { await model.setTokenContribution(false) }
                    }
                    .buttonStyle(GlassButtonStyle(.glass))
                    .disabled(model.tokenContributionBusy || model.daemonSettings?.tokenDistributionsContribution == nil)
                }
                if let storage = model.daemonSettings?.tokenStorage {
                    storageBlock(storage)
                }
                if model.tokenContributionSaveFailed {
                    GlassNotice(tone: .outside) { Text(copy.tokenSaveFailed ?? "") }
                }
            }
        }
        .opacity(copy.tokenHeading == nil ? 0 : 1)
        .disabled(copy.tokenHeading == nil)
        .accessibilityHidden(copy.tokenHeading == nil)
        .confirmationDialog((copy.tokenHeading ?? ""), isPresented: $showingTokenDisclosure, titleVisibility: .visible) {
            Button((copy.tokenConfirm ?? "")) {
                Task { await model.setTokenContribution(true, disclosureConfirmed: true) }
            }
            Button((copy.tokenCancel ?? ""), role: .cancel) { }
        } message: {
            Text([(copy.tokenDisclosure ?? ""), (copy.tokenCaptureNote ?? ""), (copy.tokenScopeNote ?? "")].joined(separator: "\n\n"))
        }
    }

    @ViewBuilder
    private func storageBlock(_ storage: TokenStorageView) -> some View {
        if let label = storage.captureLabel {
            note(storage.captureNotice ?? "")
            Button(label) {
                if storage.captureEnabled == true { Task { await model.setLocalTokenCapture(false) } }
                else { showingTokenCapture = true }
            }
            .buttonStyle(GlassButtonStyle(.glass))
            .disabled(model.tokenContributionBusy)
            .confirmationDialog(label, isPresented: $showingTokenCapture, titleVisibility: .visible) {
                Button(label) { Task { await model.setLocalTokenCapture(true) } }
                Button(storage.cancelLabel, role: .cancel) { }
            } message: { Text(storage.captureConfirmation ?? "") }
        }
        prose(storage.stateLine)
        prose(storage.scopeNote)
        HStack(spacing: GlassTokens.Space.s3) {
            Button(storage.cleanupLabel) { Task { await model.cleanTokenStorage(discard: false) } }
                .buttonStyle(GlassButtonStyle(.glass))
            Button(storage.discardLabel, role: .destructive) { showingTokenDiscard = true }
                .buttonStyle(GlassButtonStyle(.glass))
        }
        .disabled(model.tokenContributionBusy)
        .confirmationDialog(storage.discardLabel, isPresented: $showingTokenDiscard, titleVisibility: .visible) {
            Button(storage.confirmLabel, role: .destructive) { Task { await model.cleanTokenStorage(discard: true) } }
            Button(storage.cancelLabel, role: .cancel) { }
        } message: { Text(storage.discardConfirmation) }
        if !model.tokenStorageNotice.isEmpty { note(model.tokenStorageNotice) }
    }

    private func prose(_ text: String) -> some View {
        Text(text)
            .glassType(GlassTokens.TypeScale.label.weight(.regular))
            .foregroundStyle(GlassColor.textPrimary)
            .fixedSize(horizontal: false, vertical: true)
    }

    private func note(_ text: String) -> some View {
        Text(text)
            .glassType(GlassTokens.TypeScale.caption)
            .foregroundStyle(GlassColor.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
    }

    /// `WitnessTone` -> the glass status. A refusal is `.outside` and never
    /// `.ask`: ask is caution, and a refusing witness sends nothing at all.
    /// Deliberately not `ToolsSection.tone`: the two ABI tone ranges are
    /// disjoint so a cross-wired mapper is wrong for every value.
    static func tone(_ tone: WitnessTone) -> GlassStatus {
        switch tone {
        case .refused: return .outside
        case .attention, .held: return .ask
        case .clear: return .on
        case .neutral: return .off
        }
    }

    private static func tagTone(_ status: GlassStatus) -> GlassTag.Tone {
        switch status {
        case .on: return .on
        case .ask: return .ask
        case .outside: return .outside
        default: return .neutral
        }
    }
}
