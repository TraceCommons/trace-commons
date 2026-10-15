import SwiftUI
import TCDesign

/// A session's opening prompt, set as what it is (owner, 2026-10-12): what
/// the person typed as a quotation, in italics; what a tool wrapped in a tag
/// (`<pasted_content id="…">…</pasted_content>`, `<command-name>…`) as a
/// labelled block in the monospaced face, named by its tag in words. A tag
/// left open runs to the end, as a prompt cut short leaves it. The text is
/// the daemon's scrubbed preview; nothing here rewords it.
struct PromptQuote: View {
    let prompt: String?
    /// The core's line for a session with no prompt.
    let empty: String

    /// One run of the prompt: typed text, or a tagged block.
    enum Part: Equatable {
        case typed(String)
        case tagged(name: String, body: String)
    }

    /// The most lines each part shows before it is cut short.
    static let typedLines = 6
    static let taggedLines = 5

    var body: some View {
        if let prompt {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                ForEach(Array(Self.parts(prompt).enumerated()), id: \.offset) { _, part in
                    switch part {
                    case .typed(let text):
                        Text("\u{201C}\(text)\u{201D}")
                            .glassType(GlassTokens.TypeScale.body)
                            .italic()
                            .foregroundStyle(GlassColor.textPrimary)
                            .lineLimit(Self.typedLines)
                            .fixedSize(horizontal: false, vertical: true)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    case .tagged(let name, let body):
                        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                            Label(Self.words(name), systemImage: Self.icon(name))
                                .labelStyle(.titleAndIcon)
                                .glassType(GlassTokens.TypeScale.eyebrow)
                                .foregroundStyle(GlassColor.textTertiary)
                            Text(body)
                                .glassType(GlassTokens.TypeScale.mono)
                                .foregroundStyle(GlassColor.textSecondary)
                                .lineLimit(Self.taggedLines)
                                .fixedSize(horizontal: false, vertical: true)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .padding(GlassTokens.Space.s4)
                                .background(
                                    RoundedRectangle(cornerRadius: 8, style: .continuous).fill(GlassColor.ink(0.06)))
                        }
                    }
                }
            }
        } else {
            Text(empty)
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textSecondary)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    /// The prompt split into typed runs and tagged blocks, in order. Blank
    /// runs are dropped.
    static func parts(_ prompt: String) -> [Part] {
        var parts: [Part] = []
        var rest = Substring(prompt)
        func typed(_ text: Substring) {
            let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
            if !trimmed.isEmpty { parts.append(.typed(trimmed)) }
        }
        while let open = rest.range(of: #"<([A-Za-z][A-Za-z0-9_-]*)(\s[^>]*)?>"#, options: .regularExpression) {
            typed(rest[..<open.lowerBound])
            let tag = rest[open]
            let name = String(tag.dropFirst().prefix { $0.isLetter || $0.isNumber || $0 == "_" || $0 == "-" })
            let after = rest[open.upperBound...]
            if let close = after.range(of: "</\(name)>") {
                parts.append(.tagged(name: name, body: after[..<close.lowerBound].trimmingCharacters(in: .whitespacesAndNewlines)))
                rest = after[close.upperBound...]
            } else {
                parts.append(.tagged(name: name, body: after.trimmingCharacters(in: .whitespacesAndNewlines)))
                rest = ""
            }
        }
        typed(rest)
        return parts.filter {
            if case .tagged(_, let body) = $0 { return !body.isEmpty }
            return true
        }
    }

    /// A tag's name in words: "pasted_content" reads "Pasted content".
    static func words(_ name: String) -> String {
        let spaced = name.replacingOccurrences(of: "_", with: " ").replacingOccurrences(of: "-", with: " ")
        return spaced.prefix(1).uppercased() + spaced.dropFirst()
    }

    /// The block's glyph: a clipboard for pasted text, a terminal for a
    /// command, a document otherwise.
    static func icon(_ name: String) -> String {
        let lower = name.lowercased()
        if lower.contains("paste") { return "doc.on.clipboard" }
        if lower.contains("command") || lower.contains("bash") { return "terminal" }
        return "doc.text"
    }
}
