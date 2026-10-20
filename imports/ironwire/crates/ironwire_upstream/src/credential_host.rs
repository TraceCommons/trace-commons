//! Where a first-party credential may be sent (`docs/TRUST.md` I2).
//!
//! One check shared by every backend that attaches a credential it did not
//! derive from its own base URL. It parses the URL rather than slicing the
//! string: the hand-split prefix test it replaces waved through
//! `https://127.0.0.1@evil.example` (userinfo, then the real host) and
//! `https://localhost.evil.example`, and both deliver the bearer to
//! `evil.example`.

use url::{Host, Url};

use crate::backend::UpstreamError;

/// Allow a credential issued by `issuer` onto a request for `base_url` only if
/// that URL names the issuer itself over https, or this machine.
///
/// Loopback is allowed over plain http because it is where the test doubles
/// live, and the rule is about not handing a credential to a *third party*.
/// The issuer is not: a cleartext request to the real provider puts the bearer
/// on the wire for anyone in between.
pub(crate) fn check(base_url: &str, issuer: &'static str) -> Result<(), UpstreamError> {
    let refuse =
        |attempted: String| Err(UpstreamError::CredentialHostMismatch { issuer, attempted });
    let Ok(url) = Url::parse(base_url) else {
        return refuse(without_userinfo(base_url));
    };
    let Some(host) = url.host_str() else {
        return refuse(without_userinfo(base_url));
    };
    // What the message names. Built from the parsed parts so a password in the
    // URL never reaches it — this text is returned to the client.
    let place = match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    };

    // Whatever the host. No provider base URL carries one, and userinfo is
    // exactly what made the old check read the wrong host.
    if !url.username().is_empty() || url.password().is_some() {
        return refuse(format!(
            "{place} (the base URL carries a username or password)"
        ));
    }

    let loopback = matches!(url.scheme(), "http" | "https")
        && match url.host() {
            Some(Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
            Some(Host::Ipv4(ip)) => ip.is_loopback(),
            Some(Host::Ipv6(ip)) => ip.is_loopback(),
            None => false,
        };
    if loopback {
        return Ok(());
    }
    if host.eq_ignore_ascii_case(issuer) {
        if url.scheme() == "https" {
            return Ok(());
        }
        return refuse(format!("{place} (the issuer is only reached over https)"));
    }
    refuse(place)
}

/// A base URL that did not parse, as written, minus anything that could be a
/// password: everything between the scheme and the last `@` goes. Coarser
/// than real userinfo parsing on purpose — the input is malformed, so there is
/// no grammar to trust about where the secret ends.
fn without_userinfo(raw: &str) -> String {
    let (scheme, rest) = match raw.split_once("://") {
        Some((scheme, rest)) => (Some(scheme), rest),
        None => (None, raw),
    };
    let rest = rest.rfind('@').map_or(rest, |at| &rest[at + 1..]);
    match scheme {
        Some(scheme) => format!("{scheme}://{rest}"),
        None => rest.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISSUER: &str = "api.anthropic.com";

    fn attempted(base_url: &str) -> String {
        match check(base_url, ISSUER) {
            Err(UpstreamError::CredentialHostMismatch { attempted, .. }) => attempted,
            other => panic!("{base_url} should have been refused, got {other:?}"),
        }
    }

    #[test]
    fn a_base_url_that_does_not_parse_is_a_mismatch_named_as_written() {
        assert_eq!(attempted("api.anthropic.com"), "api.anthropic.com");
        assert_eq!(attempted("https://exa mple.test"), "https://exa mple.test");
        assert_eq!(attempted(""), "");
    }

    #[test]
    fn a_url_with_no_host_is_a_mismatch() {
        assert_eq!(attempted("file:///etc/passwd"), "file:///etc/passwd");
        assert_eq!(attempted("mailto:someone"), "mailto:someone");
    }

    #[test]
    fn the_refusal_never_repeats_a_password_from_the_url() {
        for base_url in [
            "https://user:hunter2@evil.example",
            "https://user:hunter2@api.anthropic.com",
            "http://user:hunter2@127.0.0.1:9",
            "https://:hunter2@evil.example/v1",
            // Unparseable (the port is not a number), so the raw value is what
            // gets named — and it must still lose the password.
            "https://user:hunter2@evil.example:notaport",
            "https://user:hunter2/x@evil.example:notaport",
        ] {
            let err = check(base_url, ISSUER).expect_err("userinfo is always refused");
            let shown = format!("{err} {err:?}");
            assert!(
                !shown.contains("hunter2"),
                "{base_url} leaked its password: {shown}"
            );
        }
    }

    #[test]
    fn the_refusal_names_where_the_credential_would_have_gone() {
        assert_eq!(attempted("https://evil.example/v1"), "https://evil.example");
        assert_eq!(
            attempted("https://evil.example:8443"),
            "https://evil.example:8443"
        );
        assert_eq!(
            attempted("https://127.0.0.1@evil.example"),
            "https://evil.example (the base URL carries a username or password)"
        );
        assert_eq!(
            attempted("http://api.anthropic.com"),
            "http://api.anthropic.com (the issuer is only reached over https)"
        );
    }

    #[test]
    fn loopback_must_still_be_a_web_url() {
        attempted("ftp://127.0.0.1/");
    }

    #[test]
    fn encoded_or_unusual_url_spellings_cannot_hide_a_foreign_host() {
        for issuer in ["api.anthropic.com", "chatgpt.com", "api.openai.com"] {
            for base in [
                format!("https://{issuer}%40evil.example/"),
                format!("https://{issuer}.evil.example/"),
                format!("https://{issuer}@evil.example/"),
                format!("https://{issuer}@evil.example\\/"),
                format!("https://user:secret@{issuer}/"),
                format!("http://{issuer}/"),
                "https://localhost%2eevil.example/".to_string(),
                "https://127.0.0.1%2eevil.example/".to_string(),
                "https://[::ffff:192.0.2.1]/".to_string(),
                "https://[::ffff:127.0.0.1]/".to_string(),
                "https://2130706434.evil.example/".to_string(),
                "https://127.1@evil.example/".to_string(),
                "https://[::1]@evil.example/".to_string(),
                "https://localhost:9@evil.example/".to_string(),
                "ftp://localhost/".to_string(),
                "file://localhost/etc/passwd".to_string(),
            ] {
                assert!(check(&base, issuer).is_err(), "{base} for {issuer}");
            }
        }
    }

    #[test]
    fn normalised_loopback_addresses_still_name_this_machine() {
        for base in [
            "http://127.1:9/",
            "http://2130706433:9/",
            "http://0x7f000001:9/",
            "http://0177.0.0.1:9/",
            "http://[0:0:0:0:0:0:0:1]:9/",
            "http://LOCALHOST:9/",
        ] {
            assert!(check(base, ISSUER).is_ok(), "{base}");
        }
    }
}
