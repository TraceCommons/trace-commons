//! Typed errors surfaced by the operator HTTP client.
//!
//! The server speaks a `{"error": "<Label>"}` envelope on non-success status.
//! Operator binaries should map each label to a human-actionable diagnostic;
//! [`Error::ServerLabel`] preserves the raw label so callers can pattern-match.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    /// Bearer-token env var not set or empty.
    #[error("{env_var} is not set; refusing to call trace-commons API without credentials")]
    BearerMissing { env_var: String },

    /// Endpoint URL did not parse.
    #[error("invalid trace-commons endpoint {endpoint}: {source}")]
    InvalidEndpoint {
        endpoint: String,
        #[source]
        source: url::ParseError,
    },

    /// Endpoint host is not on the operator's allowlist.
    #[error("trace-commons endpoint host {host} is not on the allowed-hosts list ({allowed})")]
    HostNotAllowed { host: String, allowed: String },

    /// Network or transport-level failure (DNS, TLS, timeout, connection reset).
    #[error("trace-commons API request to {url} failed: {source}")]
    Transport {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    /// Server returned non-success status WITHOUT a structured `{"error": ...}` body.
    #[error("trace-commons API request to {url} failed with {status}: {body}")]
    HttpFailure {
        url: String,
        status: reqwest::StatusCode,
        body: String,
    },

    /// Server returned non-success status WITH a typed label in the body.
    /// Binary code should match on `label` to produce audience-specific diagnostics.
    #[error("trace-commons API request to {url} failed with {status} ({label})")]
    ServerLabel {
        url: String,
        status: reqwest::StatusCode,
        label: String,
        body: String,
    },

    /// A caller-supplied request header is not a legal HTTP header value.
    ///
    /// The name is carried; the value never is. On the path this exists for
    /// -- forwarding a redaction-witness certificate -- the value is
    /// certificate material, and an error string is one `tracing::error!`
    /// away from a log line.
    #[error("request header {name} is not a legal header value")]
    HeaderMalformed { name: String },

    /// Server returned a success status but the response body didn't deserialize
    /// into the expected type.
    #[error("trace-commons API at {url} returned malformed response: {source}")]
    MalformedResponse {
        url: String,
        body: String,
        #[source]
        source: serde_json::Error,
    },
}

/// Strip query strings + fragments from a URL for safe inclusion in error
/// messages and logs. The host allowlist + bearer-token gate are enforced
/// before any request leaves the process, so credentials should never reach
/// query strings — but defense-in-depth.
pub(crate) fn sanitize_url_for_display(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut parsed) => {
            parsed.set_query(None);
            parsed.set_fragment(None);
            parsed.to_string()
        }
        Err(_) => url.split(['?', '#']).next().unwrap_or(url).to_string(),
    }
}

/// Parse a `{"error": "<Label>"}` envelope out of a non-success response body.
/// Returns `None` if the body isn't valid JSON or doesn't have a string `error`
/// field — caller falls back to [`Error::HttpFailure`].
pub(crate) fn parse_error_label(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    value
        .get("error")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

impl Error {
    /// Returns the typed server label if this is a [`Error::ServerLabel`].
    pub fn server_label(&self) -> Option<&str> {
        match self {
            Error::ServerLabel { label, .. } => Some(label.as_str()),
            _ => None,
        }
    }
}

impl Error {
    /// Convenience for tests + diagnostics: short kind tag.
    pub fn kind(&self) -> &'static str {
        match self {
            Error::BearerMissing { .. } => "bearer-missing",
            Error::InvalidEndpoint { .. } => "invalid-endpoint",
            Error::HostNotAllowed { .. } => "host-not-allowed",
            Error::Transport { .. } => "transport",
            Error::HttpFailure { .. } => "http-failure",
            Error::ServerLabel { .. } => "server-label",
            Error::MalformedResponse { .. } => "malformed-response",
            Error::HeaderMalformed { .. } => "header-malformed",
        }
    }
}

// Display-only safe accessors for use in user-facing CLI output.
impl Error {
    /// Returns a one-line user-facing diagnostic that strips query strings
    /// and bodies. Operator binaries should prefer this for `stderr` output;
    /// the full Debug form is for verbose logging only.
    pub fn user_diagnostic(&self) -> String {
        match self {
            Error::BearerMissing { env_var } => format!("credential refused: ${env_var} not set"),
            Error::InvalidEndpoint { endpoint, .. } => format!(
                "endpoint URL is malformed: {}",
                sanitize_url_for_display(endpoint)
            ),
            Error::HostNotAllowed { host, allowed } => {
                format!("host {host} blocked by allowed-hosts list ({allowed})")
            }
            // The name, never the value: on the path this exists for the
            // value is certificate material.
            Error::HeaderMalformed { name } => {
                format!("request header {name} is not a legal header value")
            }
            Error::Transport { url, source } => format!(
                "request to {} failed: {source}",
                sanitize_url_for_display(url)
            ),
            Error::HttpFailure { url, status, .. } => {
                format!("{} returned HTTP {status}", sanitize_url_for_display(url))
            }
            Error::ServerLabel {
                url, status, label, ..
            } => format!(
                "{} refused with HTTP {status} ({label})",
                sanitize_url_for_display(url)
            ),
            Error::MalformedResponse { url, .. } => format!(
                "{} returned a response the operator client could not parse",
                sanitize_url_for_display(url)
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_url_drops_query_and_fragment() {
        assert_eq!(
            sanitize_url_for_display("https://api.example/v1/traces?token=secret"),
            "https://api.example/v1/traces"
        );
        assert_eq!(
            sanitize_url_for_display("https://api.example/v1/traces#frag"),
            "https://api.example/v1/traces"
        );
        assert_eq!(
            sanitize_url_for_display("https://api.example/v1/traces?a=1&b=2#frag"),
            "https://api.example/v1/traces"
        );
    }

    #[test]
    fn sanitize_url_preserves_path_on_invalid_input() {
        assert_eq!(
            sanitize_url_for_display("not-a-url?token=secret"),
            "not-a-url"
        );
    }

    #[test]
    fn parse_error_label_extracts_label_from_well_formed_envelope() {
        let body = r#"{"error":"PilotAllowlistNotMatched"}"#;
        assert_eq!(
            parse_error_label(body).as_deref(),
            Some("PilotAllowlistNotMatched")
        );
    }

    #[test]
    fn parse_error_label_returns_none_for_non_json_body() {
        assert_eq!(parse_error_label("Internal Server Error"), None);
    }

    #[test]
    fn parse_error_label_returns_none_when_error_field_missing() {
        let body = r#"{"detail":"oops"}"#;
        assert_eq!(parse_error_label(body), None);
    }

    #[test]
    fn parse_error_label_returns_none_when_error_field_not_string() {
        let body = r#"{"error":42}"#;
        assert_eq!(parse_error_label(body), None);
    }

    #[test]
    fn server_label_accessor_returns_label_only_for_server_label_variant() {
        let labeled = Error::ServerLabel {
            url: "https://api.example/v1/traces".into(),
            status: reqwest::StatusCode::FORBIDDEN,
            label: "PilotAllowlistNotMatched".into(),
            body: "{}".into(),
        };
        assert_eq!(labeled.server_label(), Some("PilotAllowlistNotMatched"));

        let bearer = Error::BearerMissing {
            env_var: "TRACE_COMMONS_REVIEWER_BEARER".into(),
        };
        assert_eq!(bearer.server_label(), None);
    }

    #[test]
    fn user_diagnostic_strips_query_strings() {
        let err = Error::HttpFailure {
            url: "https://api.example/v1/traces?token=secret".into(),
            status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            body: "...".into(),
        };
        let diag = err.user_diagnostic();
        assert!(
            !diag.contains("token=secret"),
            "diagnostic leaked query string: {diag}"
        );
        assert!(diag.contains("https://api.example/v1/traces"));
    }

    #[test]
    fn kind_tags_are_stable() {
        let cases = [
            (
                Error::BearerMissing {
                    env_var: "X".into(),
                },
                "bearer-missing",
            ),
            (
                Error::HostNotAllowed {
                    host: "evil.example".into(),
                    allowed: "api.example".into(),
                },
                "host-not-allowed",
            ),
        ];
        for (err, want) in cases {
            assert_eq!(err.kind(), want);
        }
    }
}
