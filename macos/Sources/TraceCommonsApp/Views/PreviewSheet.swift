import SwiftUI
import TCBridge

/// "Look inside": the one surface in the product that deliberately shows
/// trace content, because consent to send something you cannot see is not
/// consent.
///
/// Four tabs, in the spec's order. **Search is first and focused** on
/// purpose: "does this mention my client's name?" is a question a
/// contributor can answer in five seconds. Judging redaction quality by eye
/// is not, and this interface never asks them to.
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

    @State private var current: QueueEntry
    @State private var remaining: [QueueEntry] = []
    @State private var preview: TCPreview?
    @State private var summary: PreviewSummary?
    @State private var transcriptText: String
    @State private var failure: String?
    @State private var loading: Bool
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
    }

    init(entry: QueueEntry, preloaded: Preloaded? = nil) {
        self.entry = entry
        self.preloaded = preloaded
        _current = State(initialValue: entry)
        _summary = State(initialValue: preloaded?.summary)
        _transcriptText = State(initialValue: preloaded?.transcript ?? "")
        _loading = State(initialValue: preloaded == nil)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            Divider()
            content
            Divider()
            footer
        }
        .frame(width: 820, height: 620)
        .task(id: current.entryID) {
            guard preloaded == nil else { return }
            await load()
        }
        .onAppear {
            remaining = model.awaitingDecision.filter { $0.entryID != current.entryID }
        }
        .onDisappear { closePreview() }
    }

    // MARK: - Chrome

    private var header: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                Text(current.projectLabel).font(.headline)
                Text(current.agentName).foregroundStyle(.secondary)
                Text("·").foregroundStyle(.tertiary)
                Text(Format.when(current.discoveredAt)).foregroundStyle(.secondary)
                Spacer()
                if let summary {
                    Text("Would send \(Format.bytes(summary.wouldSendBytes))")
                        .foregroundStyle(.secondary)
                }
            }
            .font(.callout)
            Text("Nothing has been sent. This is what would be.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(16)
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
            // A segmented picker rather than a TabView: inside a sheet this
            // is the standard macOS treatment, and Search has to be able to
            // start selected and focused.
            VStack(alignment: .leading, spacing: 12) {
                Picker("", selection: $tab) {
                    ForEach(Tab.allCases) { tab in
                        Text(tab.title).tag(tab)
                    }
                }
                .pickerStyle(.segmented)
                .labelsHidden()

                switch tab {
                case .search:
                    SearchTab(
                        transcript: transcriptText,
                        preview: preview,
                        initialNeedle: preloaded?.needle ?? "",
                        initialOffsets: preloaded?.offsets
                    )
                case .whatsInIt:
                    WhatsInItTab(entry: current, summary: summary)
                case .transcript:
                    TranscriptTab(transcript: transcriptText)
                case .permissions:
                    PermissionsTab(summary: summary, options: model.consentScopes)
                }
                Spacer(minLength: 0)
            }
            .padding(12)
        }
    }

    private var footer: some View {
        HStack {
            Button("Not this one") {
                model.dismiss(current)
                advance()
            }
            Spacer()
            if !remaining.isEmpty {
                Text("\(remaining.count) more after this")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            Button("Close") { dismiss() }
            // The ONLY approve control in the product, and it is behind the
            // preview by design.
            Button("Contribute") {
                model.approve(current)
                advance()
            }
            .keyboardShortcut(.defaultAction)
            .disabled(summary == nil)
        }
        .padding(16)
    }

    // MARK: - Flow

    /// Advancing to the next entry inside the sheet is what makes three
    /// sessions three deliberate clicks in one flow. There is no select-all.
    private func advance() {
        closePreview()
        guard let next = remaining.first else {
            dismiss()
            return
        }
        remaining.removeFirst()
        summary = nil
        transcriptText = ""
        failure = nil
        loading = true
        current = next
    }

    private func load() async {
        loading = true
        let outcome = await model.openPreview(entryID: current.entryID)
        switch outcome {
        case .opened(let opened):
            preview = opened
            transcriptText = opened.body
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

// MARK: - Tabs

/// The highest-value affordance in the product: type a client name, get
/// `0 matches` or jump-to-context, without reading 148 turns.
struct SearchTab: View {
    let transcript: String
    let preview: TCPreview?

    @State private var needle: String
    @State private var offsets: [Int]?
    @State private var searched: Bool
    @State private var recents: [String] = RecentSearches.load()
    @FocusState private var focused: Bool

    init(
        transcript: String,
        preview: TCPreview?,
        initialNeedle: String = "",
        initialOffsets: [Int]? = nil
    ) {
        self.transcript = transcript
        self.preview = preview
        _needle = State(initialValue: initialNeedle)
        _offsets = State(initialValue: initialOffsets)
        _searched = State(initialValue: initialOffsets != nil)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Search this trace for anything you need to be sure isn't in it.")
                .font(.callout)
            TextField("Client name, hostname, anything", text: $needle)
                .textFieldStyle(.roundedBorder)
                .focused($focused)
                .onSubmit(run)
                .onChange(of: needle) { _, _ in run() }

            if !recents.isEmpty {
                HStack(spacing: 6) {
                    Text("Recent:").font(.caption).foregroundStyle(.secondary)
                    ForEach(recents, id: \.self) { term in
                        Button(term) { needle = term }
                            .buttonStyle(.link)
                            .font(.caption)
                    }
                }
            }

            resultSummary

            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    ForEach(Array(contexts.enumerated()), id: \.offset) { _, snippet in
                        Text(snippet)
                            .font(.system(.callout, design: .monospaced))
                            .textSelection(.enabled)
                            .padding(8)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 6))
                    }
                }
            }
        }
        .onAppear { focused = true }
    }

    @ViewBuilder
    private var resultSummary: some View {
        if !searched || needle.isEmpty {
            Text("Type to search. Nothing is sent while you look.")
                .font(.callout)
                .foregroundStyle(.secondary)
        } else if offsets == nil {
            Text("The search couldn't run on this trace.")
                .font(.callout)
                .foregroundStyle(.secondary)
        } else if offsets!.isEmpty {
            Text("0 matches")
                .font(.title3.weight(.semibold))
                .foregroundStyle(.green)
        } else {
            Text("^[\(offsets!.count) match](inflect: true)")
                .font(.title3.weight(.semibold))
                .foregroundStyle(.orange)
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

    /// The ABI reports UTF-8 BYTE offsets, so context is cut from the byte
    /// array and decoded back, never from Swift's character indices.
    private var contexts: [String] {
        guard let offsets, !offsets.isEmpty else { return [] }
        let bytes = Array(transcript.utf8)
        let window = 120
        return offsets.prefix(20).map { offset in
            let start = max(0, offset - window)
            let end = min(bytes.count, offset + needle.utf8.count + window)
            guard start < end else { return "" }
            let slice = Array(bytes[start..<end])
            let text = String(decoding: slice, as: UTF8.self)
                .replacingOccurrences(of: "\n", with: " ")
            return (start > 0 ? "…" : "") + text + (end < bytes.count ? "…" : "")
        }
    }
}

struct WhatsInItTab: View {
    let entry: QueueEntry
    let summary: PreviewSummary

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 10) {
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
                .font(.caption)
                .foregroundStyle(.secondary)

                Divider()
                Text("What scrubbing removed").font(.headline)
                if summary.redactions.isEmpty {
                    Text("""
                    Nothing matched. On a session that touched credentials, that is \
                    itself worth a second look.
                    """)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                } else {
                    ForEach(summary.redactions.sorted(by: { $0.key < $1.key }), id: \.key) { kind, count in
                        Text("\(count) × \(kind.replacingOccurrences(of: "_", with: " "))")
                            .font(.callout)
                    }
                }

                if !summary.piiLabelsPresent.isEmpty {
                    Divider()
                    Text("Personal-information categories seen").font(.headline)
                    Text(summary.piiLabelsPresent.joined(separator: ", "))
                        .font(.callout)
                    Text("Categories only. The matched text is never reported here.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }

                Divider()
                Text("Residual risk: \(summary.residualRisk.replacingOccurrences(of: "_", with: " "))")
                    .font(.callout)
                Text("""
                Files touched and tools invoked are not in this contract's preview \
                summary, so they are not shown rather than guessed at.
                """)
                .font(.caption)
                .foregroundStyle(.tertiary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(4)
        }
    }
}

/// The redacted transcript exactly as it would be sent. Redactions stay
/// visible as inline chips (`[SECRET]`, `[PATH]`) rather than deletions, so a
/// contributor can see WHERE scrubbing fired -- which is the point.
struct TranscriptTab: View {
    let transcript: String

    var body: some View {
        ScrollView([.vertical, .horizontal]) {
            Text(transcript.isEmpty ? "(empty)" : transcript)
                .font(.system(.caption, design: .monospaced))
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(6)
        }
    }
}

/// The consent scopes this upload will carry, restated at the moment of
/// consent rather than only at onboarding.
struct PermissionsTab: View {
    let summary: PreviewSummary
    let options: [ConsentScope]

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text("What this upload asks for").font(.headline)
                ForEach(summary.consentScopes, id: \.self) { scope in
                    VStack(alignment: .leading, spacing: 2) {
                        Text(ScopeCopy.title(for: scope, options: options))
                            .font(.callout.weight(.semibold))
                        if let description = options.first(where: { $0.name == scope })?.description {
                            Text(description).font(.callout).foregroundStyle(.secondary)
                        }
                    }
                }
                Text("""
                These are the permissions this device requests. Trace Commons can \
                narrow them, never widen them -- and if your permissions change \
                between now and sending, this approval stops applying and you are \
                asked again.
                """)
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(4)
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

enum RecentSearches {
    private static let key = "trace-commons.recent-searches"

    static func load() -> [String] {
        UserDefaults.standard.stringArray(forKey: key) ?? []
    }

    /// Recent searches persist so the second trace is one keystroke. They
    /// are the contributor's own words, kept locally, and never sent.
    static func remember(_ term: String) -> [String] {
        var terms = load().filter { $0 != term }
        terms.insert(term, at: 0)
        terms = Array(terms.prefix(6))
        UserDefaults.standard.set(terms, forKey: key)
        return terms
    }
}
