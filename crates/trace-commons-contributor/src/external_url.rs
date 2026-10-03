//! Which outside URLs the app may hand to the OS to open in a browser (K7,
//! #1173), moved out of Tauri so every shell opens only the same allowed
//! hosts. The list is exactly Tauri's `open_external_url` allowlist, kept
//! as it was -- no widening.
//!
//! This governs ONLY the general "open this in your browser" surface. It
//! has nothing to do with the single-use, state-consuming wallet and
//! account-sign-in URLs a shell itself minted (Tauri's
//! `open_native_wallet_url` / `open_account_sign_in_url`): those are
//! validated against a nonce the shell holds, not against a fixed host
//! list, and stay in each shell.

/// Whether `url` is one of the fixed destinations the app is allowed to
/// hand to the OS opener: the near.ai credits dashboard, a tracecommons.ai
/// public run, the CI fixture commit on GitHub, or a loopback OAuth
/// callback.
///
/// Checks the whole string -- scheme, host, path, and a length/charset
/// sanity check -- not only the host: a host match on a string that also
/// carries control characters, embedded whitespace, or a userinfo
/// (`user@host`) component is not a safe thing to shell out to
/// `open`/`xdg-open`/`explorer.exe` with, even though each per-destination
/// check below also re-checks its own authority.
pub fn is_allowed(url: &str) -> bool {
    let allowed_origin = near_credits_url_is_allowed(url)
        || tracecommons_run_url_is_allowed(url)
        || tracecommons_fixture_url_is_allowed(url)
        || loopback_callback_url_is_allowed(url);
    allowed_origin
        && url.len() <= 2048
        && url.is_ascii()
        && !url
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
}

/// A loopback OAuth redirect on an ephemeral port, used by the NEAR AI
/// credential ceremony's local callback listener.
fn loopback_callback_url_is_allowed(url: &str) -> bool {
    let Some(authority_and_path) = url.strip_prefix("http://") else {
        return false;
    };
    let authority_end = authority_and_path
        .find(['/', '?', '#'])
        .unwrap_or(authority_and_path.len());
    let authority = &authority_and_path[..authority_end];
    let Some(port) = authority.strip_prefix("127.0.0.1:") else {
        return false;
    };
    !port.is_empty()
        && port.bytes().all(|byte| byte.is_ascii_digit())
        && port.parse::<u16>().is_ok_and(|port| port > 0)
}

/// The near.ai credits dashboard for one organisation.
fn near_credits_url_is_allowed(url: &str) -> bool {
    let prefix = "https://cloud.near.ai/dashboard/organizations/";
    let Some(rest) = url.strip_prefix(prefix) else {
        return false;
    };
    let Some(organization_id) = rest.strip_suffix("/credits") else {
        return false;
    };
    !organization_id.is_empty()
        && organization_id.len() <= 128
        && organization_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// A tracecommons.ai public run page, named by its slug alone.
fn tracecommons_run_url_is_allowed(url: &str) -> bool {
    let prefix = "https://tracecommons.ai/runs/";
    let Some(slug) = url.strip_prefix(prefix) else {
        return false;
    };
    !slug.is_empty()
        && slug.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

/// The CI fixture commit on GitHub, named by its full 40-hex-digit SHA.
fn tracecommons_fixture_url_is_allowed(url: &str) -> bool {
    let prefix = "https://github.com/TraceCommons/trace-commons/commit/";
    let Some(commit) = url.strip_prefix(prefix) else {
        return false;
    };
    commit.len() == 40 && commit.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_allowed_destinations_pass() {
        assert!(is_allowed(
            "http://127.0.0.1:49152/near-ai/callback?state=abc"
        ));
        assert!(is_allowed(
            "https://cloud.near.ai/dashboard/organizations/example/credits"
        ));
        assert!(is_allowed(
            "https://tracecommons.ai/runs/repair-a-stalled-upload"
        ));
        assert!(is_allowed(
            "https://github.com/TraceCommons/trace-commons/commit/b6722426bb4b83d90425494b664ac468d67943b5"
        ));
    }

    #[test]
    fn a_userinfo_component_is_refused_even_on_an_allowed_port() {
        assert!(!is_allowed(
            "http://127.0.0.1:49152@attacker.example/near-ai/callback"
        ));
    }

    #[test]
    fn a_port_outside_u16_range_is_refused() {
        assert!(!is_allowed("http://127.0.0.1:99999/near-ai/callback"));
    }

    #[test]
    fn a_zero_port_is_refused() {
        assert!(!is_allowed("http://127.0.0.1:0/near-ai/callback"));
    }

    #[test]
    fn near_credits_rejects_a_query_string_a_path_traversal_and_an_empty_id() {
        assert!(near_credits_url_is_allowed(
            "https://cloud.near.ai/dashboard/organizations/example/credits"
        ));
        assert!(!near_credits_url_is_allowed(
            "https://cloud.near.ai/dashboard/organizations/example/credits?next=https://example.com"
        ));
        assert!(!near_credits_url_is_allowed(
            "https://cloud.near.ai/dashboard/organizations/../other/credits"
        ));
        assert!(!near_credits_url_is_allowed(
            "https://cloud.near.ai/dashboard/organizations//credits"
        ));
    }

    #[test]
    fn tracecommons_run_rejects_a_query_string_and_uppercase() {
        assert!(tracecommons_run_url_is_allowed(
            "https://tracecommons.ai/runs/repair-a-stalled-upload"
        ));
        assert!(!tracecommons_run_url_is_allowed(
            "https://tracecommons.ai/runs/repair-a-stalled-upload?next=https://example.com"
        ));
        assert!(!tracecommons_run_url_is_allowed(
            "https://tracecommons.ai/runs/Repair-A-Stalled-Upload"
        ));
    }

    #[test]
    fn tracecommons_fixture_rejects_a_query_string_wrong_case_and_wrong_length() {
        assert!(tracecommons_fixture_url_is_allowed(
            "https://github.com/TraceCommons/trace-commons/commit/b6722426bb4b83d90425494b664ac468d67943b5"
        ));
        assert!(!tracecommons_fixture_url_is_allowed(
            "https://github.com/TraceCommons/trace-commons/commit/b6722426bb4b83d90425494b664ac468d67943b5?next=https://example.com"
        ));
        assert!(!tracecommons_fixture_url_is_allowed(
            "https://github.com/tracecommons/trace-commons/commit/b6722426bb4b83d90425494b664ac468d67943b5"
        ));
        assert!(!tracecommons_fixture_url_is_allowed(
            "https://github.com/TraceCommons/trace-commons/commit/b6722426"
        ));
    }

    /// A lookalike host -- a subdomain, a suffix, or a userinfo-bearing
    /// authority that a naive `contains`/`starts_with` check might accept --
    /// is refused by every one of the per-destination checks.
    #[test]
    fn lookalike_hosts_are_refused() {
        for url in [
            "https://evil.cloud.near.ai/dashboard/organizations/example/credits",
            "https://cloud.near.ai.evil.example/dashboard/organizations/example/credits",
            "https://notcloud.near.ai/dashboard/organizations/example/credits",
            "https://evil.tracecommons.ai/runs/repair-a-stalled-upload",
            "https://tracecommons.ai.evil.example/runs/repair-a-stalled-upload",
            "https://github.com.evil.example/TraceCommons/trace-commons/commit/b6722426bb4b83d90425494b664ac468d67943b5",
            "https://evilgithub.com/TraceCommons/trace-commons/commit/b6722426bb4b83d90425494b664ac468d67943b5",
        ] {
            assert!(!is_allowed(url), "{url}");
        }
    }

    /// `javascript:`, `data:`, and a plain `http://` (not loopback) to one
    /// of the allowed hostnames are refused: the scheme is part of the
    /// match, not stripped off before comparison.
    #[test]
    fn non_https_schemes_to_an_allowed_host_are_refused() {
        for url in [
            "javascript:alert(document.cookie)",
            "javascript://cloud.near.ai/dashboard/organizations/example/credits",
            "data:text/html,<script>alert(1)</script>",
            "http://cloud.near.ai/dashboard/organizations/example/credits",
            "http://tracecommons.ai/runs/repair-a-stalled-upload",
            "file:///etc/passwd",
        ] {
            assert!(!is_allowed(url), "{url}");
        }
    }

    /// Over-long and garbage input is refused quickly, without panicking,
    /// exactly like a short one -- length is checked, not merely implied by
    /// the prefix/suffix matches above failing to find room for it.
    #[test]
    fn over_long_and_garbage_input_is_refused() {
        let huge = format!("https://tracecommons.ai/runs/{}", "a".repeat(3000));
        assert!(!is_allowed(&huge));
        assert!(!is_allowed(&"x".repeat(100_000)));
        assert!(!is_allowed(""));
        assert!(!is_allowed("\u{0}\u{0}\u{0}"));
    }

    /// Embedded whitespace or a control character defeats the match even
    /// when the rest of the string is an otherwise-allowed URL -- a shell
    /// that reflows such a string before shelling out to `open` must not be
    /// handed one that looks allowed until it does.
    #[test]
    fn embedded_whitespace_and_control_characters_are_refused() {
        assert!(!is_allowed(
            "https://cloud.near.ai/dashboard/organizations/example/credits\nopen"
        ));
        assert!(!is_allowed(
            "https://tracecommons.ai/runs/repair-a-stalled\t-upload"
        ));
        assert!(!is_allowed(
            "https://cloud.near.ai/dashboard/organizations/example/credits\u{7}"
        ));
    }

    /// Non-ASCII (e.g. a homoglyph/IDN host) is refused outright, not
    /// normalised and re-checked.
    #[test]
    fn non_ascii_is_refused() {
        assert!(!is_allowed(
            "https://tracecommons.ai/runs/répair-a-stalled-upload"
        ));
    }
}
