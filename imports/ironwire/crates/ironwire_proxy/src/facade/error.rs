//! Turning IronWire failures into errors the client already knows how to
//! handle.
//!
//! `docs/DESIGN.md` §9: a request that cannot be served returns a
//! protocol-correct error with the provider's own `retry-after` preserved.
//! Coding agents have well-tested paths for a 429 or a 529; they have no path
//! at all for an IronWire-shaped error, and inventing one turns a recoverable
//! stall into a crashed session.

use axum::Json;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use ironwire_core::policy::{NoRoute, Refusal};
use ironwire_core::protocol::BackendId;
use ironwire_upstream::backend::UpstreamError;
use serde_json::json;

use crate::pipeline::PipelineError;

/// An error rendered in a provider's own error shape.
#[derive(Debug)]
pub struct FacadeError {
    status: StatusCode,
    kind: &'static str,
    message: String,
    retry_after_secs: Option<u64>,
    /// A line for Codex's own limit banner. See
    /// [`FacadeError::on_openai_facade`].
    codex_notice: Option<String>,
    /// Whether to render OpenAI's error envelope rather than Anthropic's.
    openai_envelope: bool,
}

/// The one response header Codex renders as free text.
///
/// Codex parses `x-codex-promo-message` on a usage-limit response and prints it
/// inside its own sentence: "You've hit your usage limit. <message>, or try
/// again later." Verified against Codex 0.145 — the matching `promo_message`
/// field in the body is *not* read, only this header.
///
/// It is the only channel a proxy has into that UI, and this is the one moment
/// worth using it: the user has just been stopped, and what they need is what
/// IronWire tried and what is left — which otherwise lives in a terminal they
/// are not looking at.
const CODEX_NOTICE_HEADER: &str = "x-codex-promo-message";

impl FacadeError {
    /// Build directly.
    #[must_use]
    pub fn new(status: StatusCode, kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            kind,
            message: message.into(),
            retry_after_secs: None,
            codex_notice: None,
            openai_envelope: false,
        }
    }

    /// The status this will be returned with.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// A malformed request body.
    #[must_use]
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request_error", message)
    }

    /// A login-bound URL naming a login IronWire does not have.
    ///
    /// 404, because the URL names a resource that is not there, and a status
    /// every client already treats as final: retrying will not make the login
    /// exist. Each envelope spells it the way its provider does — Anthropic as
    /// `not_found_error`, OpenAI as `invalid_request_error`.
    #[must_use]
    pub fn unknown_login(message: impl Into<String>, openai: bool) -> Self {
        let error = Self::new(
            StatusCode::NOT_FOUND,
            if openai {
                "invalid_request_error"
            } else {
                "not_found_error"
            },
            message,
        );
        if openai {
            error.on_openai_facade(false)
        } else {
            error
        }
    }

    /// Render this error the way the OpenAI façade's clients expect it.
    ///
    /// The envelope is not cosmetic. Every error here was leaving on the
    /// *Anthropic* shape regardless of which façade it came from, so a Codex
    /// client received `{"type":"error",…}`, failed to recognise it, and
    /// retried a hopeless request five times before giving up with a message
    /// about retries rather than about capacity.
    ///
    /// For a rate limit from a client that identified as Codex it goes
    /// further, into the one shape Codex renders specially — and then into the
    /// only text channel a proxy has into that UI. Verified against Codex
    /// 0.145: `error.type = "usage_limit_reached"` produces the limit banner,
    /// and `x-codex-promo-message` supplies the clause inside it ("You've hit
    /// your usage limit. <ours>, or try again later."). Hence a clause: no
    /// capital, no full stop.
    ///
    /// Nothing is invented to get there. Codex's own limit responses also carry
    /// a plan type and a reset time; those are the provider's facts, we do not
    /// have them, and the banner renders without them.
    #[must_use]
    pub fn on_openai_facade(mut self, client_is_codex: bool) -> Self {
        self.openai_envelope = true;
        if client_is_codex && self.status == StatusCode::TOO_MANY_REQUESTS {
            self.kind = "usage_limit_reached";
            // ASCII only: this rides in a header, and `HeaderValue` rejects
            // anything above 0x7f — an em dash here silently drops the whole
            // notice, which looks exactly like the channel not working.
            self.codex_notice = Some(
                "IronWire had no capacity left on any other connected pool either; \
                 run `ironwire status` to see when each returns"
                    .to_string(),
            );
        }
        self
    }

    /// Render a pipeline failure.
    #[must_use]
    pub fn from_pipeline(error: &PipelineError) -> Self {
        Self::from_pipeline_naming(error, &|_| None)
    }

    /// [`Self::from_pipeline`], with what to run to connect each login the
    /// refusal names.
    ///
    /// The router knows a subscription belongs to another login, but not where
    /// that login lives, so its refusal can only say `--config-dir DIR`. The
    /// façade can ask the backend, and a command the user can paste is the
    /// difference between an instruction and a riddle.
    #[must_use]
    pub fn from_pipeline_naming(
        error: &PipelineError,
        connect: &dyn Fn(&BackendId) -> Option<String>,
    ) -> Self {
        match error {
            PipelineError::NoRoute(no_route) => Self::from_no_route(no_route, connect),
            PipelineError::Admission(_) => Self::invalid_request(error.to_string()),
            PipelineError::Upstream(upstream) | PipelineError::AllFailed { last: upstream, .. } => {
                Self::from_upstream(upstream)
            }
        }
    }

    fn from_no_route(no_route: &NoRoute, connect: &dyn Fn(&BackendId) -> Option<String>) -> Self {
        match no_route {
            NoRoute::NoBackendsConfigured => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "api_error",
                "IronWire has no backends configured. Run `ironwire connect claude`.",
            ),
            NoRoute::AllExhausted => Self::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limit_error",
                "Every connected backend is rate limited or unavailable. \
                 Run `ironwire status` to see when capacity returns.",
            ),
            NoRoute::NoneUsable { refusals } => {
                // Every backend that was considered, in the order the router
                // put them in — most actionable first. Listing only the ones
                // with an interesting reason is what made this message point at
                // the wrong backend: a wire mismatch is worth saying, and it is
                // never the sentence a user can act on.
                let detail = refusals
                    .iter()
                    .map(|(id, why)| match (why, connect(id)) {
                        (Refusal::AnotherLogin { .. }, Some(command)) => {
                            format!("{id}: {} (for this login: `{command}`)", why.describe())
                        }
                        _ => format!("{id}: {}", why.describe()),
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                Self::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "api_error",
                    format!(
                        "No connected backend could take this request. {detail}. \
                         IronWire refuses rather than silently degrading; \
                         run `ironwire status` for the full picture."
                    ),
                )
            }
            // The caller named a backend explicitly, so this is a *request*
            // error, not a capacity one — 400, and it lists what exists so the
            // answer is actionable rather than just a refusal.
            NoRoute::UnknownRoute {
                requested,
                available,
            } => Self::new(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                format!(
                    "X-IronWire-Route asked for `{requested}`, which is not a \
                     connected backend. Available: {}.",
                    if available.is_empty() {
                        "none".to_string()
                    } else {
                        available.join(", ")
                    }
                ),
            ),
            // 429 rather than 503: the client's own back-off is the right
            // behaviour, and the window really does reopen — at midnight.
            NoRoute::SpendCapReached {
                backend,
                spent_usd,
                cap_usd,
            } => Self::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limit_error",
                format!(
                    "Stopped by your own spend cap: {backend} has spent \
                     ${spent_usd:.2} of its ${cap_usd:.2} daily limit, and \
                     `[limits] on_breach = \"refuse\"` in config.toml says to stop \
                     rather than fall back. Raise the cap, switch to \
                     `on_breach = \"descend\"`, or wait for the window to reset."
                ),
            ),
            // The cause is named first and plainly: a user who forgot the
            // setting has to be able to connect the symptom to it in one read.
            // It says what IronWire is *doing* — restricting routing to a named
            // set — and never that anything is safe (`docs/TRUST.md` I7).
            NoRoute::NoTrustedBackendAvailable { tried, missing } => {
                let mut detail = String::new();
                if !tried.is_empty() {
                    detail.push_str(&format!(
                        " Trusted backends were tried: {}.",
                        tried
                            .iter()
                            .map(|(id, why)| format!("{id} ({why})"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if !missing.is_empty() {
                    detail.push_str(&format!(
                        " These ids are in `trusted_backends` but are not \
                         connected backends: {}.",
                        missing.join(", ")
                    ));
                }
                Self::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "api_error",
                    format!(
                        "Refused because `privacy.mode = \"full\"` restricts routing \
                         to the backends in `privacy.trusted_backends`, and none of \
                         them can serve this request.{detail} IronWire will not fall \
                         back to a backend you excluded. Run `ironwire status` to \
                         see the trusted set, or change the mode in config.toml."
                    ),
                )
            }
            NoRoute::RequiresClientIdentity => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "api_error",
                "The only available capacity is a subscription reserved for its own \
                 client. IronWire does not present one product's identity to unlock \
                 another's subscription. Connect an API key with \
                 `ironwire connect anthropic-api`.",
            ),
        }
    }

    fn from_upstream(error: &UpstreamError) -> Self {
        match error {
            UpstreamError::RateLimited {
                retry_after_secs, ..
            } => Self {
                status: StatusCode::TOO_MANY_REQUESTS,
                kind: "rate_limit_error",
                message: error.to_string(),
                // The provider's own delay, never one of ours.
                retry_after_secs: *retry_after_secs,
                codex_notice: None,
                openai_envelope: false,
            },
            UpstreamError::NeedsAuth { .. } => Self::new(
                StatusCode::UNAUTHORIZED,
                "authentication_error",
                error.to_string(),
            ),
            UpstreamError::Transport { .. } => {
                Self::new(StatusCode::BAD_GATEWAY, "api_error", error.to_string())
            }
            UpstreamError::Upstream { status, .. } => Self::new(
                StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
                "api_error",
                error.to_string(),
            ),
            UpstreamError::CredentialHostMismatch { .. } => Self::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "api_error",
                error.to_string(),
            ),
        }
    }
}

impl IntoResponse for FacadeError {
    fn into_response(self) -> Response {
        let mut headers = HeaderMap::new();
        if let Some(secs) = self.retry_after_secs
            && let Ok(value) = HeaderValue::from_str(&secs.to_string())
        {
            headers.insert("retry-after", value);
        }
        if let Some(notice) = &self.codex_notice
            && let Ok(value) = HeaderValue::from_str(notice)
        {
            headers.insert(CODEX_NOTICE_HEADER, value);
        }
        // Each façade's own error envelope, so an IronWire failure arrives as
        // a failure the client already has a path for — which is this module's
        // whole reason to exist.
        let body = if self.openai_envelope {
            json!({ "error": { "type": self.kind, "message": self.message } })
        } else {
            json!({
                "type": "error",
                "error": { "type": self.kind, "message": self.message },
            })
        };
        (self.status, headers, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironwire_core::capability::Ineligible;

    #[test]
    fn a_rate_limit_reaches_the_client_as_a_rate_limit() {
        // Claude Code backs off correctly on a 429; it has no handling at all
        // for a bespoke IronWire status.
        let err =
            FacadeError::from_pipeline(&PipelineError::Upstream(UpstreamError::RateLimited {
                backend: BackendId::from("claude-sub"),
                retry_after_secs: Some(30),
            }));
        assert_eq!(err.status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(err.retry_after_secs, Some(30));
        assert_eq!(err.kind, "rate_limit_error");
    }

    /// Codex reads this header only on a usage-limit response, so attaching it
    /// anywhere else is noise on a path that already has an explanation.
    #[test]
    fn the_codex_notice_rides_only_on_a_limit_response() {
        let limited = FacadeError::from_pipeline(&PipelineError::NoRoute(NoRoute::AllExhausted))
            .on_openai_facade(true);
        assert!(limited.codex_notice.is_some());

        let other =
            FacadeError::from_pipeline(&PipelineError::NoRoute(NoRoute::NoBackendsConfigured))
                .on_openai_facade(true);
        assert!(other.codex_notice.is_none());
    }

    /// A client that did not identify as Codex is not shown a Codex banner —
    /// and would not render it anyway.
    #[test]
    fn a_non_codex_client_gets_no_codex_notice() {
        let err = FacadeError::from_pipeline(&PipelineError::NoRoute(NoRoute::AllExhausted))
            .on_openai_facade(false);
        assert!(err.codex_notice.is_none());
    }

    /// The notice travels as a header value, and `HeaderValue` refuses
    /// anything non-ASCII. An em dash in this string dropped the header
    /// entirely, which is indistinguishable from the channel not existing.
    #[test]
    fn the_notice_survives_being_a_header_value() {
        let err = FacadeError::from_pipeline(&PipelineError::NoRoute(NoRoute::AllExhausted))
            .on_openai_facade(true);
        let notice = err.codex_notice.clone().expect("attached");
        assert!(notice.is_ascii(), "not header-safe: {notice}");
        assert!(HeaderValue::from_str(&notice).is_ok());

        let response = err.into_response();
        assert!(
            response.headers().contains_key(CODEX_NOTICE_HEADER),
            "the notice never reached the response"
        );
    }

    /// Codex prints the message inside a sentence of its own, so ours has to
    /// read as a clause rather than start a new one.
    #[test]
    fn the_notice_reads_as_a_clause() {
        let err = FacadeError::from_pipeline(&PipelineError::NoRoute(NoRoute::AllExhausted))
            .on_openai_facade(true);
        let notice = err.codex_notice.expect("attached");
        assert!(!notice.ends_with('.'), "got: {notice}");
        assert!(
            notice.starts_with(|c: char| !c.is_uppercase()) || notice.starts_with("IronWire"),
            "got: {notice}"
        );
    }

    #[test]
    fn we_never_invent_a_retry_after() {
        let err =
            FacadeError::from_pipeline(&PipelineError::Upstream(UpstreamError::RateLimited {
                backend: BackendId::from("claude-sub"),
                retry_after_secs: None,
            }));
        assert_eq!(err.retry_after_secs, None);
    }

    #[test]
    fn an_ineligible_route_explains_itself_rather_than_degrading() {
        let err = FacadeError::from_pipeline(&PipelineError::NoRoute(NoRoute::NoneUsable {
            refusals: vec![(
                BackendId::from("nearai"),
                Refusal::Ineligible(Ineligible::MidToolLoop),
            )],
        }));
        assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(err.message.contains("nearai"));
        assert!(err.message.contains("MidToolLoop"));
    }

    /// The failure this message was rewritten for. A machine with two
    /// subscriptions nobody has consented to and one backend on the wrong wire
    /// used to be told only about the wire — the two credentials sitting right
    /// there never appeared, and neither did the only instruction that would
    /// have fixed it.
    #[test]
    fn a_refusal_names_every_backend_it_considered() {
        let err = FacadeError::from_pipeline(&PipelineError::NoRoute(NoRoute::NoneUsable {
            refusals: vec![
                (BackendId::from("codex-sub"), Refusal::NotConsented),
                (BackendId::from("claude-sub"), Refusal::NotConsented),
                (
                    BackendId::from("nearai"),
                    Refusal::Ineligible(Ineligible::ImagesUnsupported),
                ),
            ],
        }));
        for backend in ["codex-sub", "claude-sub", "nearai"] {
            assert!(err.message.contains(backend), "{} is missing", backend);
        }
        assert!(err.message.contains("consent"));
        // The actionable one leads, because it is the sentence with something
        // to do in it.
        let consent = err.message.find("codex-sub").expect("listed");
        let wire = err.message.find("nearai").expect("listed");
        assert!(consent < wire, "the wire mismatch is leading the message");
    }

    #[test]
    fn the_identity_refusal_tells_the_user_what_to_do_about_it() {
        let err =
            FacadeError::from_pipeline(&PipelineError::NoRoute(NoRoute::RequiresClientIdentity));
        assert!(err.message.contains("ironwire connect anthropic-api"));
    }

    #[test]
    fn an_unconfigured_daemon_says_so_instead_of_timing_out() {
        let err =
            FacadeError::from_pipeline(&PipelineError::NoRoute(NoRoute::NoBackendsConfigured));
        assert!(err.message.contains("ironwire connect claude"));
    }
}
