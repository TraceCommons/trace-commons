import SwiftUI

/// The core supplies the sentence and glyph; the shell maps its semantic tone.
struct NativeFlowNotice: View {
    let message: String
    let glyph: String
    let tone: String
    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: TC.Space.s) {
            if !glyph.isEmpty { Text(glyph) }
            Text(message).fixedSize(horizontal: false, vertical: true)
        }
        .font(TC.Font_.meta)
        .foregroundStyle(tone == "refused" ? TC.Tone.refused.textColor : TC.Tone.neutral.textColor)
        .accessibilityElement(children: .combine)
    }
}
