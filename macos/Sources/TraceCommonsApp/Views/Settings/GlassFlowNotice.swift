import SwiftUI
import TCDesign

/// The glass counterpart of `NativeFlowNotice`: the core supplies the
/// sentence, the glyph and the tone; the shell only maps the tone onto a
/// glass status. The tone colours the words themselves, because a
/// `GlassNotice` without a title draws no status dot.
struct GlassFlowNotice: View {
    let message: String
    let glyph: String
    /// The core's tone; nil when its copy could not be read, which is
    /// drawn as a refusal, since only a failure is ever shown here.
    let tone: String?

    /// The core's refusal tone, or no tone at all, is the outside status;
    /// any other tone is drawn as plain, never as healthy.
    static func status(forTone tone: String?) -> GlassStatus {
        tone == nil || tone == "refused" ? .outside : .off
    }

    var body: some View {
        let status = Self.status(forTone: tone)
        // A refusal reads in the outside text colour; anything else stays in
        // the notice's own text colour, as legacy's neutral tone did.
        let ink = status == .outside ? status.textColor : GlassColor.textSecondary
        GlassNotice(tone: status) {
            HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s2) {
                if !glyph.isEmpty { Text(glyph) }
                Text(message).fixedSize(horizontal: false, vertical: true)
            }
            .foregroundStyle(ink)
            .accessibilityElement(children: .combine)
        }
    }
}
