//! Parsing for the `tracecommons://` URL scheme and the other launch
//! arguments the shells deliver (K7, #1173), moved out of Tauri so every
//! shell -- Tauri today, macOS over the C ABI -- parses identically.
//!
//! [`parse_deep_link`] is a PURE function: it never opens a browser, never
//! stores a pending link, and never acts on what it finds. A caller decides
//! what the returned [`DeepLinkAction`] means. A link this module does not
//! recognise, or cannot parse safely, is refused with [`DEEP_LINK_INVALID`]
//! -- there is no "best effort" interpretation of an unknown or malformed
//! link, and the two are refused identically so nothing downstream can
//! branch on "malformed" versus "unknown".

use serde::Serialize;

use crate::commands::invite_from_deep_link;

/// The link that opens the review queue. A digest notification's click is
/// routed to it in-process by a shell that already holds this exact string,
/// rather than round-tripping through the OS.
pub const REVIEW_DEEP_LINK: &str = "tracecommons://review";

/// The refusal label [`parse_deep_link`] returns for anything it does not
/// recognise, including a malformed link. Stable: a shell may show this
/// string, or compare against it, but must not invent its own.
pub const DEEP_LINK_INVALID: &str = "deep-link-invalid";

/// What a parsed deep link asks the caller to do. The `kind` tag and each
/// variant's fields are exactly the wire shape Tauri's `consume_deep_link`
/// has always returned (see `tauri-desktop/frontend/src/lib/tauri/
/// platform-api.ts`) -- this is the core taking over the logic that
/// produced it, not a new contract.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeepLinkAction {
    /// `tracecommons://enroll?invite=<percent-encoded-invite-url>`.
    Enroll { invite: String },
    /// `tracecommons://run/<slug>`, naming the public run page.
    PublicRun { slug: String, url: String },
    /// `tracecommons://credential[/<provider>]`, defaulting to `near_ai`.
    Credential { provider: String },
    /// [`REVIEW_DEEP_LINK`] itself: navigate to the review queue.
    Navigate { path: &'static str },
}

/// Parse one deep link or launch argument. `Ok` names exactly one action;
/// `Err` is always [`DEEP_LINK_INVALID`].
pub fn parse_deep_link(url: &str) -> Result<DeepLinkAction, &'static str> {
    if let Some(invite) = enroll_invite(url) {
        return Ok(DeepLinkAction::Enroll { invite });
    }
    if let Some(slug) = public_run_slug(url) {
        return Ok(DeepLinkAction::PublicRun {
            url: format!("https://tracecommons.ai/runs/{slug}"),
            slug,
        });
    }
    if let Some(provider) = credential_provider(url) {
        return Ok(DeepLinkAction::Credential { provider });
    }
    if is_review_link(url) {
        return Ok(DeepLinkAction::Navigate { path: "/waiting" });
    }
    Err(DEEP_LINK_INVALID)
}

/// Whether `url` opens with the app's own URL scheme, compared
/// case-insensitively the way LaunchServices and the Windows shell both
/// deliver it.
pub fn is_tracecommons_deep_link(url: &str) -> bool {
    url.get(..15)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("tracecommons://"))
}

/// Split a `tracecommons://` link into its authority (the part between `//`
/// and the next `/`, `?` or `#`) and everything after, refusing a
/// scheme-only prefix with embedded whitespace/control characters, an empty
/// or `@`-bearing authority, or an authority holding anything outside
/// `[A-Za-z0-9_-]` -- which also excludes IDN/punycode hosts and anything
/// that could be read as a second, nested scheme.
fn deep_link_parts(url: &str) -> Option<(&str, &str)> {
    if !is_tracecommons_deep_link(url) {
        return None;
    }
    let rest = &url[15..];
    if rest
        .chars()
        .any(|character| character.is_control() || character.is_whitespace())
    {
        return None;
    }
    let boundary = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..boundary];
    if authority.is_empty()
        || authority.contains('@')
        || !authority
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return None;
    }
    Some((authority, &rest[boundary..]))
}

/// A length/charset sanity check applied to the invite URL folded into an
/// `enroll` link, on top of [`invite_from_deep_link`]'s own scheme/host/key
/// parse: a known scheme and host with an oversized or control-bearing
/// payload is refused the same as an unknown one.
fn invite_link_is_valid(value: &str) -> bool {
    if value.len() > 2048
        || !value.is_ascii()
        || value
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return false;
    }
    let Some(rest) = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
    else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') || !authority.is_ascii() {
        return false;
    }
    let host = match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && !port.is_empty() => {
            if port.parse::<u16>().is_err() {
                return false;
            }
            host
        }
        Some(_) => return false,
        None => authority,
    };
    if host.is_empty()
        || !host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return false;
    }
    let fragment_code = value
        .split_once('#')
        .is_some_and(|(_, code)| !code.is_empty());
    let query_code = value
        .split_once('?')
        .and_then(|(_, query)| query.split('#').next())
        .into_iter()
        .flat_map(|query| query.split('&'))
        .any(|pair| {
            let Some((name, code)) = pair.split_once('=') else {
                return false;
            };
            name == "code" && !code.is_empty()
        });
    fragment_code || query_code
}

/// The invite folded into an `enroll` link. The outer link goes through
/// [`deep_link_parts`] first, like every other arm: [`invite_from_deep_link`]
/// parses with a WHATWG URL parser, which trims leading/trailing whitespace,
/// deletes tabs and newlines anywhere, and accepts a userinfo or a port on
/// the authority -- each of which this module refuses rather than
/// normalising away.
fn enroll_invite(url: &str) -> Option<String> {
    let (authority, _) = deep_link_parts(url)?;
    if !authority.eq_ignore_ascii_case("enroll") {
        return None;
    }
    let invite = invite_from_deep_link(url)?;
    invite_link_is_valid(&invite).then_some(invite)
}

fn public_run_slug(url: &str) -> Option<String> {
    let (authority, rest) = deep_link_parts(url)?;
    if !authority.eq_ignore_ascii_case("run") || !rest.starts_with('/') {
        return None;
    }
    let slug = &rest[1..];
    if slug.is_empty()
        || slug.len() > 63
        || !slug
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        || !slug
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        || slug.contains('/')
        || slug.contains('?')
        || slug.contains('#')
        || !slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return None;
    }
    Some(slug.to_owned())
}

fn credential_provider(url: &str) -> Option<String> {
    let (authority, rest) = deep_link_parts(url)?;
    if !authority.eq_ignore_ascii_case("credential") {
        return None;
    }
    if rest.contains('?') || rest.contains('#') {
        return None;
    }
    let provider = rest.strip_prefix('/').unwrap_or_default();
    if provider.contains('/')
        || (!provider.is_empty()
            && !provider
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_'))
    {
        return None;
    }
    Some(if provider.is_empty() {
        "near_ai".to_owned()
    } else {
        provider.to_owned()
    })
}

fn is_review_link(url: &str) -> bool {
    deep_link_parts(url).is_some_and(|(authority, rest)| {
        authority.eq_ignore_ascii_case("review") && rest.is_empty()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enroll_links_yield_their_invite() {
        assert_eq!(
            parse_deep_link(
                "tracecommons://enroll?invite=https%3A%2F%2Fissuer.example%2Fonboard%23CODE"
            ),
            Ok(DeepLinkAction::Enroll {
                invite: "https://issuer.example/onboard#CODE".to_owned()
            })
        );
        assert_eq!(
            parse_deep_link(
                "TraceCommons://ENROLL/?invite=https%3A%2F%2Fissuer.example%2Fonboard%23CODE"
            ),
            Ok(DeepLinkAction::Enroll {
                invite: "https://issuer.example/onboard#CODE".to_owned()
            })
        );
        // The exact argv a real Windows shell delivered: LaunchServices'
        // normalisation added a slash after the host that nothing in our
        // code wrote. Parsing via `Url` survives it.
        assert_eq!(
            parse_deep_link(
                "tracecommons://enroll/?invite=https%3A%2F%2Fissuer.tracecommons.ai%2Fonboard%23VQWWPGYSG8Y4LTP6"
            ),
            Ok(DeepLinkAction::Enroll {
                invite: "https://issuer.tracecommons.ai/onboard#VQWWPGYSG8Y4LTP6".to_owned()
            })
        );
    }

    #[test]
    fn an_empty_invite_value_is_refused() {
        assert_eq!(
            parse_deep_link("tracecommons://enroll?invite="),
            Err(DEEP_LINK_INVALID)
        );
    }

    /// The two invite refusals Tauri's own test pinned before the move: an
    /// `invite` under any authority but `enroll`, and an invite URL that
    /// carries no code in either its fragment or a `code=` query pair.
    #[test]
    fn an_invite_under_another_authority_or_without_a_code_is_refused() {
        assert_eq!(
            parse_deep_link(
                "tracecommons://other?invite=https%3A%2F%2Fissuer.example%2Fonboard%23CODE"
            ),
            Err(DEEP_LINK_INVALID)
        );
        assert_eq!(
            parse_deep_link("tracecommons://enroll?invite=https%3A%2F%2Fissuer.example%2Fonboard"),
            Err(DEEP_LINK_INVALID)
        );
    }

    /// The enroll arm is held to the same outer-link rules as every other
    /// arm: whitespace or a control character anywhere in the link, a
    /// userinfo component, or a port on the authority is refused, never
    /// trimmed or normalised away by a lenient URL parser first.
    #[test]
    fn enroll_links_get_the_same_outer_link_checks_as_every_other_arm() {
        for arg in [
            " tracecommons://enroll?invite=https%3A%2F%2Fissuer.example%2Fonboard%23CODE",
            "tracecommons://enroll?invite=https%3A%2F%2Fissuer.example%2Fonboard%23CODE\n",
            "tracecommons://en\troll?invite=https%3A%2F%2Fissuer.example%2Fonboard%23CODE",
            "tracecommons://enroll?invite=https%3A%2F%2Fissuer.ex\nample%2Fonboard%23CODE",
            "tracecommons://user@enroll?invite=https%3A%2F%2Fissuer.example%2Fonboard%23CODE",
            "tracecommons://enroll:443?invite=https%3A%2F%2Fissuer.example%2Fonboard%23CODE",
        ] {
            assert_eq!(parse_deep_link(arg), Err(DEEP_LINK_INVALID), "{arg:?}");
        }
    }

    #[test]
    fn an_oversized_invite_is_refused_even_with_a_known_scheme_and_host() {
        let huge = format!(
            "tracecommons://enroll?invite=https%3A%2F%2Fissuer.example%2F{}",
            "a".repeat(2100)
        );
        assert_eq!(parse_deep_link(&huge), Err(DEEP_LINK_INVALID));
    }

    #[test]
    fn public_run_links_yield_the_slug_and_a_built_url() {
        assert_eq!(
            parse_deep_link("tracecommons://run/repair-a-stalled-upload"),
            Ok(DeepLinkAction::PublicRun {
                slug: "repair-a-stalled-upload".to_owned(),
                url: "https://tracecommons.ai/runs/repair-a-stalled-upload".to_owned(),
            })
        );
        assert_eq!(
            parse_deep_link("TraceCommons://run/repair-a-stalled-upload"),
            Ok(DeepLinkAction::PublicRun {
                slug: "repair-a-stalled-upload".to_owned(),
                url: "https://tracecommons.ai/runs/repair-a-stalled-upload".to_owned(),
            })
        );
    }

    #[test]
    fn public_run_slugs_reject_case_leading_dash_and_trailers() {
        assert_eq!(
            parse_deep_link("tracecommons://run/-repair-a-stalled-upload"),
            Err(DEEP_LINK_INVALID)
        );
        assert_eq!(
            parse_deep_link("tracecommons://run/Repair-a-stalled-upload"),
            Err(DEEP_LINK_INVALID)
        );
        assert_eq!(
            parse_deep_link("tracecommons://run/repair-a-stalled-upload?next=evil"),
            Err(DEEP_LINK_INVALID)
        );
    }

    #[test]
    fn credential_links_default_to_near_ai_or_name_their_provider() {
        assert_eq!(
            parse_deep_link("tracecommons://credential"),
            Ok(DeepLinkAction::Credential {
                provider: "near_ai".to_owned()
            })
        );
        assert_eq!(
            parse_deep_link("tracecommons://credential/near_ai"),
            Ok(DeepLinkAction::Credential {
                provider: "near_ai".to_owned()
            })
        );
        assert_eq!(
            parse_deep_link("tracecommons://credential/near_ai?token=secret"),
            Err(DEEP_LINK_INVALID)
        );
        assert_eq!(
            parse_deep_link("tracecommons://credential?token=secret"),
            Err(DEEP_LINK_INVALID)
        );
    }

    #[test]
    fn the_review_link_navigates_to_waiting() {
        assert_eq!(
            parse_deep_link(REVIEW_DEEP_LINK),
            Ok(DeepLinkAction::Navigate { path: "/waiting" })
        );
        assert_eq!(
            parse_deep_link("TraceCommons://REVIEW"),
            Ok(DeepLinkAction::Navigate { path: "/waiting" })
        );
        assert_eq!(
            parse_deep_link("tracecommons://review?next=/settings"),
            Err(DEEP_LINK_INVALID)
        );
    }

    #[test]
    fn other_launch_arguments_are_refused_not_guessed() {
        // Registering a scheme handler means this is asked about every
        // argument the shell is ever launched with, including its own.
        for arg in [
            "https://example.com/",
            "tracecommons://open?x=1",
            "--state-dir",
            "",
            "tracecommons:",
            "tracecommons:///",
        ] {
            assert_eq!(parse_deep_link(arg), Err(DEEP_LINK_INVALID), "{arg}");
        }
    }

    /// `javascript:` and other non-`tracecommons` schemes are refused, not
    /// merely unmatched by any arm -- a link whose scheme is not this app's
    /// own never reaches a host/path comparison at all.
    #[test]
    fn non_tracecommons_schemes_are_refused() {
        for arg in [
            "javascript:alert(1)",
            "javascript://review",
            "file:///etc/passwd",
            "data:text/html,review",
        ] {
            assert_eq!(parse_deep_link(arg), Err(DEEP_LINK_INVALID), "{arg}");
        }
    }

    /// A lookalike authority -- a second `tracecommons` host nested in the
    /// path, or a userinfo-bearing authority that would read as one host to
    /// a naive splitter and another to a strict one -- is refused.
    #[test]
    fn lookalike_authorities_are_refused() {
        for arg in [
            "tracecommons://review@evil.example",
            "tracecommons://review.evil.example",
            "tracecommons://evil.example/review",
            "tracecommons://REVIEW%00",
        ] {
            assert_eq!(parse_deep_link(arg), Err(DEEP_LINK_INVALID), "{arg}");
        }
    }

    /// Control characters and whitespace embedded in an otherwise
    /// well-formed link are refused rather than silently trimmed.
    #[test]
    fn control_characters_and_whitespace_are_refused() {
        assert_eq!(
            parse_deep_link("tracecommons://review\nextra"),
            Err(DEEP_LINK_INVALID)
        );
        assert_eq!(
            parse_deep_link("tracecommons:// review"),
            Err(DEEP_LINK_INVALID)
        );
        assert_eq!(
            parse_deep_link("tracecommons://cre\u{7}dential"),
            Err(DEEP_LINK_INVALID)
        );
    }

    /// Over-long garbage with no recognisable structure is refused quickly
    /// and without panicking, exactly like a short one.
    #[test]
    fn over_long_garbage_is_refused() {
        let garbage = format!("tracecommons://{}", "x".repeat(100_000));
        assert_eq!(parse_deep_link(&garbage), Err(DEEP_LINK_INVALID));
        let garbage = "\u{0}".repeat(10_000);
        assert_eq!(parse_deep_link(&garbage), Err(DEEP_LINK_INVALID));
    }

    #[test]
    fn deep_link_action_serializes_to_tauris_long_standing_wire_shape() {
        assert_eq!(
            serde_json::to_value(DeepLinkAction::Enroll {
                invite: "https://issuer.example/onboard#CODE".to_owned()
            })
            .unwrap(),
            serde_json::json!({"kind": "enroll", "invite": "https://issuer.example/onboard#CODE"})
        );
        assert_eq!(
            serde_json::to_value(DeepLinkAction::PublicRun {
                slug: "repair-a-stalled-upload".to_owned(),
                url: "https://tracecommons.ai/runs/repair-a-stalled-upload".to_owned(),
            })
            .unwrap(),
            serde_json::json!({
                "kind": "public_run",
                "slug": "repair-a-stalled-upload",
                "url": "https://tracecommons.ai/runs/repair-a-stalled-upload",
            })
        );
        assert_eq!(
            serde_json::to_value(DeepLinkAction::Credential {
                provider: "near_ai".to_owned()
            })
            .unwrap(),
            serde_json::json!({"kind": "credential", "provider": "near_ai"})
        );
        assert_eq!(
            serde_json::to_value(DeepLinkAction::Navigate { path: "/waiting" }).unwrap(),
            serde_json::json!({"kind": "navigate", "path": "/waiting"})
        );
    }
}
