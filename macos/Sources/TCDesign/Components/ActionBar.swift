import SwiftUI

/// A window's action bar with its notes directly above it (owner ruling,
/// 2026-10-08).
///
/// The action bar is the footer row that holds the primary CTA; every
/// button in it is the CTA's size (`GlassButtonSize.bar`). Any general page
/// or window text about taking that action -- what it does, what it does
/// not authorize, why it waits -- sits here, directly above the bar, as
/// plain secondary text, full width and left-aligned, never in the
/// scrolling content. One placement, so every screen puts such text in the
/// same place at the same spacing rather than positioning its own.
///
/// ```swift
/// GlassActionBar(notes: [copy.noSharing]) {
///     HStack { Button("Back") {}.buttonStyle(GlassButtonStyle(.glass, size: .bar)); Spacer(); ... }
/// }
/// ```
public struct GlassActionBar<Bar: View>: View {
    /// The gap between the notes and the bar under them.
    public static var noteGap: CGFloat { GlassTokens.Space.s4 }
    /// The gap between two notes.
    public static var noteSpacing: CGFloat { GlassTokens.Space.s2 }

    private let notes: [String]
    private let bar: Bar

    /// `notes` are drawn in order, top to bottom; empty strings are skipped.
    public init(notes: [String] = [], @ViewBuilder bar: () -> Bar) {
        self.notes = notes.filter { !$0.isEmpty }
        self.bar = bar()
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: Self.noteGap) {
            if !notes.isEmpty {
                VStack(alignment: .leading, spacing: Self.noteSpacing) {
                    ForEach(Array(notes.enumerated()), id: \.offset) { _, note in
                        GlassActionNote(note)
                    }
                }
            }
            bar
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// One note above an action bar: plain secondary text, full width,
/// left-aligned. Used through `GlassActionBar`.
public struct GlassActionNote: View {
    private let text: String

    public init(_ text: String) {
        self.text = text
    }

    public var body: some View {
        Text(text)
            .glassType(GlassTokens.TypeScale.label)
            .foregroundStyle(GlassColor.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}
