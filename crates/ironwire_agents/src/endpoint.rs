//! The URLs IronWire points a client at, and recognising them again.
//!
//! Two shapes per façade. The plain one, `/anthropic` and `/openai/v1`, belongs
//! to the default login of its product. The login-bound one,
//! `/anthropic/account/{id}` and `/openai/account/{id}/v1`, tells the daemon
//! which login the client is using, and therefore which subscription pays for
//! it. Writing and reading both live here so that `connect`, `is_wired` and
//! `doctor` cannot drift into three opinions about what counts as ours.

/// Where a URL of ours points: the daemon's port, and the login it binds the
/// client to — `None` for the plain façade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// The loopback port.
    pub port: u16,
    /// The backend id from `/account/{id}`, if the URL names one.
    pub account: Option<String>,
}

/// The base URL for an Anthropic-speaking client. Claude Code appends
/// `/v1/messages` itself.
#[must_use]
pub fn anthropic_url(port: u16, account: Option<&str>) -> String {
    match account {
        None => format!("http://127.0.0.1:{port}/anthropic"),
        Some(id) => format!("http://127.0.0.1:{port}/anthropic/account/{id}"),
    }
}

/// The base URL for an OpenAI-speaking client. Codex appends `/responses`, so
/// this one carries the `/v1`.
#[must_use]
pub fn openai_url(port: u16, account: Option<&str>) -> String {
    match account {
        None => format!("http://127.0.0.1:{port}/openai/v1"),
        Some(id) => format!("http://127.0.0.1:{port}/openai/account/{id}/v1"),
    }
}

/// Read an Anthropic base URL back, if it is one of ours.
///
/// Loopback and our own path, at any port: a port change still counts as ours,
/// which is what lets `connect` follow the daemon to a new port instead of
/// giving up. Anything else is another proxy or a deliberate choice.
#[must_use]
pub fn parse_anthropic(url: &str) -> Option<Endpoint> {
    let (port, path) = loopback(url)?;
    let account = match path {
        "anthropic" => None,
        _ => Some(account_id(path.strip_prefix("anthropic/account/")?)?),
    };
    Some(Endpoint { port, account })
}

/// Read an OpenAI base URL back, if it is one of ours.
#[must_use]
pub fn parse_openai(url: &str) -> Option<Endpoint> {
    let (port, path) = loopback(url)?;
    let account = match path {
        "openai/v1" => None,
        _ => Some(account_id(
            path.strip_prefix("openai/account/")?.strip_suffix("/v1")?,
        )?),
    };
    Some(Endpoint { port, account })
}

/// Split `http://127.0.0.1:PORT/path/` into the port and `path`.
fn loopback(url: &str) -> Option<(u16, &str)> {
    let rest = url
        .strip_prefix("http://127.0.0.1:")
        .or_else(|| url.strip_prefix("http://localhost:"))?;
    let (port, path) = rest.trim_end_matches('/').split_once('/')?;
    if port.is_empty() || !port.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((port.parse().ok()?, path))
}

/// A backend id, as `Config::validate` allows them. Anything else in that
/// segment is not a URL we wrote.
fn account_id(segment: &str) -> Option<String> {
    (!segment.is_empty()
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    .then(|| segment.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_forms_read_back_as_what_was_written() {
        for account in [None, Some("claude-work")] {
            let url = anthropic_url(8463, account);
            assert_eq!(
                parse_anthropic(&url),
                Some(Endpoint {
                    port: 8463,
                    account: account.map(str::to_string)
                }),
                "{url}"
            );
            let url = openai_url(9000, account);
            assert_eq!(
                parse_openai(&url),
                Some(Endpoint {
                    port: 9000,
                    account: account.map(str::to_string)
                }),
                "{url}"
            );
        }
    }

    #[test]
    fn a_trailing_slash_and_localhost_are_still_ours() {
        assert!(parse_anthropic("http://localhost:1234/anthropic/").is_some());
        assert!(parse_anthropic("http://127.0.0.1:1234/anthropic/account/work/").is_some());
        assert!(parse_openai("http://localhost:1234/openai/account/codex-work/v1/").is_some());
    }

    /// Someone else's proxy on localhost is still theirs, and a segment we
    /// would never have written is not a login of ours.
    #[test]
    fn a_url_we_did_not_write_is_not_ours() {
        for url in [
            "http://127.0.0.1:8082",
            "http://127.0.0.1:8082/v1",
            "https://api.anthropic.com",
            "http://127.0.0.1:notaport/anthropic",
            "http://127.0.0.1:99999/anthropic",
            "http://127.0.0.1:8463/anthropic/account/",
            "http://127.0.0.1:8463/anthropic/account/a%20b",
            "http://127.0.0.1:8463/anthropic/account/work/v1",
            "http://10.0.0.2:8463/anthropic",
        ] {
            assert_eq!(parse_anthropic(url), None, "{url}");
        }
        for url in [
            "http://127.0.0.1:8463/openai",
            "http://127.0.0.1:8463/openai/account/codex-work",
            "http://127.0.0.1:8463/openai/account//v1",
            "https://chatgpt.com/backend-api/codex",
        ] {
            assert_eq!(parse_openai(url), None, "{url}");
        }
    }
}
