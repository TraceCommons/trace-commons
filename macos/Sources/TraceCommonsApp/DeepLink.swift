import Foundation

// MARK: - Deep link

/// Parses `tracecommons://enroll?invite=<percent-encoded-invite-url>`.
///
/// Mail clients cannot be made to open an arbitrary `https://` link in this
/// app, so an invite email carries the app's own URL scheme instead, with
/// the real invite link (an issuer URL, not this app's) folded into the
/// `invite` query parameter.
enum DeepLink {
    static func inviteURL(from url: URL) -> String? {
        guard url.scheme?.lowercased() == "tracecommons",
              url.host?.lowercased() == "enroll",
              let components = URLComponents(url: url, resolvingAgainstBaseURL: false)
        else { return nil }
        // The empty value is dropped, not passed on. `tracecommons://enroll?invite=`
        // used to yield Some("") here and drive the screen into a resolve of
        // nothing, with an empty field and an error. Rust filters it
        // (`commands.rs`, `.filter(|v| !v.is_empty())`) and Windows returns
        // null (`DeepLink.cs`); one invite mail reaches all three clients, so
        // the parse is a contract and this was the one place it diverged.
        return components.queryItems?
            .first(where: { $0.name == "invite" })?
            .value
            .flatMap { $0.isEmpty ? nil : $0 }
    }
}
