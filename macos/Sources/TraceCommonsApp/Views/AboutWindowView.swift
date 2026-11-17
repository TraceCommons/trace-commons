import AppKit
import SwiftUI
import TCDesign

/// The app's About window, opened from the app menu in place of the
/// standard panel (owner, 2026-11-09). It shows the standard panel's lines,
/// read from the bundle the same way (the localized name, the version with
/// its build, the copyright), and a Copy button that copies the version
/// line exactly as shown, for a bug report or a support thread.
struct AboutWindowView: View {
    var body: some View {
        VStack(spacing: GlassTokens.Space.s3) {
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .frame(width: 96, height: 96)
                .accessibilityHidden(true)
            Text(AboutLines.name)
                .glassType(GlassTokens.TypeScale.title)
                .foregroundStyle(GlassColor.textPrimary)
            HStack(spacing: GlassTokens.Space.s3) {
                Text(AboutLines.version)
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textPrimary)
                    .textSelection(.enabled)
                Button(SettingsLegacyWords.copy) { AboutLines.copyVersion() }
                    .buttonStyle(GlassButtonStyle(.glass, small: true))
            }
            if let copyright = AboutLines.copyright {
                Text(copyright)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
            }
        }
        .padding(.horizontal, GlassTokens.Space.s8)
        .padding(.top, GlassTokens.Space.s6)
        .padding(.bottom, GlassTokens.Space.s8)
        .frame(minWidth: 280)
    }
}

/// The About window's lines, from the bundle's Info.plist: what the
/// standard About panel would show.
enum AboutLines {
    /// The localized `CFBundleName` (InfoPlist.strings), as the standard
    /// panel reads it.
    static var name: String {
        string("CFBundleName") ?? ProcessInfo.processInfo.processName
    }

    /// "<version> (<build>)", the standard panel's form.
    static var version: String {
        line(short: string("CFBundleShortVersionString"), build: string("CFBundleVersion"))
    }

    static var copyright: String? { string("NSHumanReadableCopyright") }

    /// The version line in the standard panel's form; just the version
    /// when the build is missing or the same.
    static func line(short: String?, build: String?) -> String {
        let short = short ?? ""
        guard let build, !build.isEmpty, build != short else { return short }
        return short.isEmpty ? build : "\(short) (\(build))"
    }

    static func copyVersion() {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(version, forType: .string)
    }

    private static func string(_ key: String) -> String? {
        (Bundle.main.object(forInfoDictionaryKey: key) as? String).flatMap { $0.isEmpty ? nil : $0 }
    }
}
