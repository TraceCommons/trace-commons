import Foundation

/// What the local scrubber removes, for the screen that answers "what gets
/// removed?".
///
/// ## The list is generated, never written here
///
/// The names come from `tc_scrub_detector_names`, which reports the
/// scrubber's own detector table. A list transcribed into Swift would be a
/// privacy claim that silently stops being true the day a detector is added,
/// and nothing in this app would fail when it did -- the screen would simply
/// keep describing an older build. Only the PRETTIFICATION is a lookup here,
/// and an unrecognized slug still renders, de-slugged, so a detector added
/// upstream can never vanish from a screen whose whole job is to tell a
/// contributor what is scrubbed.
///
/// The patterns are deliberately not available to this layer and must not be
/// added: publishing the regexes would tell someone trying to slip a secret
/// past the scrubber exactly what to avoid.
public enum ScrubDetectors {
    /// Decode the export's JSON array into slugs, preserving order.
    ///
    /// Returns an empty array on malformed input rather than throwing. The
    /// caller is the first screen a contributor sees, and it has a truthful
    /// fallback: the residual-risk concession, which is the honest half of
    /// this screen anyway.
    public static func slugs(fromJSON json: String) -> [String] {
        guard let data = json.data(using: .utf8),
              let decoded = try? JSONDecoder().decode([String].self, from: data)
        else { return [] }
        return decoded
    }

    /// A human label for a detector slug.
    ///
    /// `everyDetectorHasAHumanLabel` in the tests fails the build if a
    /// detector arrives without one, so the de-slugged fallback is a safety
    /// net rather than the plan.
    public static func label(for slug: String) -> String {
        switch slug {
        case "openai_api_key": return "OpenAI API keys"
        case "github_token": return "GitHub tokens"
        case "aws_access_key": return "AWS access keys"
        // The regex behind this one covers Stripe, GitLab and Slack prefixes.
        // Naming them beats "provider tokens", which tells a contributor
        // nothing about whether their own provider is covered.
        case "provider_token": return "Stripe, GitLab and Slack tokens"
        case "jwt": return "JSON Web Tokens"
        case "npm_token": return "npm tokens"
        case "google_api_key": return "Google API keys"
        case "pem_header_orphan": return "Private keys in PEM blocks"
        default: return slug.replacingOccurrences(of: "_", with: " ")
        }
    }

    /// The labels to show, in the order the scrubber reports them.
    public static func labels(fromJSON json: String) -> [String] {
        slugs(fromJSON: json).map(label(for:))
    }
}
