import SwiftUI
import TCDesign

/// The core supplies the sentence and glyph; the shell maps its semantic tone.
///
/// Refused is the outside tone and anything else asks. The sentence is the
/// notice's title, so the status dot and the words are one element and the
/// state is never colour alone; the core's glyph is not drawn because that
/// dot already marks the tone.
struct NativeFlowNotice: View {
    let message: String
    let glyph: String
    let tone: String

    var body: some View {
        GlassNotice(tone: tone == "refused" ? .outside : .ask, title: message) {
            EmptyView()
        }
    }
}
