//! Minimal TraceCommons server database configuration.

use secrecy::{ExposeSecret, SecretString};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SslMode {
    Disable,
    #[default]
    Prefer,
    Require,
}

impl std::fmt::Display for SslMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disable => write!(f, "disable"),
            Self::Prefer => write!(f, "prefer"),
            Self::Require => write!(f, "require"),
        }
    }
}

impl std::str::FromStr for SslMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "disable" => Ok(Self::Disable),
            "prefer" => Ok(Self::Prefer),
            "require" => Ok(Self::Require),
            _ => Err(format!(
                "invalid DATABASE_SSLMODE '{}', expected 'disable', 'prefer', or 'require'",
                s
            )),
        }
    }
}

impl SslMode {
    pub fn from_env() -> Self {
        std::env::var("DATABASE_SSLMODE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    pub url: SecretString,
    pub pool_size: usize,
    pub ssl_mode: SslMode,
    /// Separate connection string for the narrow login-resolver pool. Its DB
    /// user MUST be the operator-provisioned `trace_login_resolver` login role
    /// (NOLOGIN base, no BYPASSRLS, column-scoped SELECT on `trace_login_links`
    /// only). Optional at the type level; the account-redeem path is
    /// fail-closed without it. Never reuse the runtime `url` here.
    pub login_resolver_url: Option<SecretString>,
}

impl DatabaseConfig {
    pub fn from_postgres_url(url: &str, pool_size: usize) -> Self {
        Self {
            url: SecretString::from(url.to_string()),
            pool_size,
            ssl_mode: SslMode::from_env(),
            login_resolver_url: Self::login_resolver_url_from_env(),
        }
    }

    /// Read the optional separate resolver connection string from
    /// `TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL`. A blank value is treated as
    /// unset so the feature stays fail-closed rather than building a pool from
    /// an empty string.
    pub fn login_resolver_url_from_env() -> Option<SecretString> {
        std::env::var("TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(SecretString::from)
    }

    pub fn url(&self) -> &str {
        self.url.expose_secret()
    }

    pub fn login_resolver_url(&self) -> Option<&str> {
        self.login_resolver_url
            .as_ref()
            .map(|value| value.expose_secret())
    }
}

/// WebAuthn relying-party configuration for the passkey ceremonies (Slice 2).
///
/// All three fields are required together. The loader is fail-closed: it returns
/// `Some` only when every one of `TRACE_COMMONS_WEBAUTHN_RP_ID`,
/// `TRACE_COMMONS_WEBAUTHN_RP_ORIGIN`, and `TRACE_COMMONS_WEBAUTHN_RP_NAME` is set
/// to a non-blank value. A partial configuration (some-but-not-all set) is treated
/// as a misconfiguration and yields `None`, so the passkey surface stays disabled
/// (its accessor fails closed) rather than building a relying party from incomplete
/// state. Mirrors `DatabaseConfig::login_resolver_url_from_env`: blank is unset.
#[derive(Debug, Clone)]
pub struct WebauthnConfig {
    /// Relying-party id (an effective domain, e.g. `tracecommons.ai`). Credentials
    /// bind to this value; it cannot change without invalidating every passkey.
    pub rp_id: String,
    /// Relying-party origin (a full URL, e.g. `https://app.tracecommons.ai`).
    pub rp_origin: String,
    /// Human-readable relying-party name shown in authenticator prompts.
    pub rp_name: String,
}

impl WebauthnConfig {
    /// Load the relying-party config from the environment. Returns `Some` only when
    /// all three of `TRACE_COMMONS_WEBAUTHN_RP_ID`, `_RP_ORIGIN`, and `_RP_NAME` are
    /// present and non-blank. A partial set fails closed (`None`).
    pub fn from_env() -> Option<Self> {
        let rp_id = Self::non_blank_env("TRACE_COMMONS_WEBAUTHN_RP_ID");
        let rp_origin = Self::non_blank_env("TRACE_COMMONS_WEBAUTHN_RP_ORIGIN");
        let rp_name = Self::non_blank_env("TRACE_COMMONS_WEBAUTHN_RP_NAME");
        match (rp_id, rp_origin, rp_name) {
            (Some(rp_id), Some(rp_origin), Some(rp_name)) => Some(Self {
                rp_id,
                rp_origin,
                rp_name,
            }),
            // Partial config is a misconfiguration: fail closed rather than build a
            // relying party from incomplete state. A fully-unset config is the
            // normal "passkeys disabled" state and is silent; a SOME-but-not-all
            // config is almost certainly an operator mistake, so warn loudly (by
            // env-var NAME only — never values; an origin/rp_id may be sensitive)
            // so it is not silently inert.
            (rp_id, rp_origin, rp_name) => {
                let any_set = rp_id.is_some() || rp_origin.is_some() || rp_name.is_some();
                if any_set {
                    let name_state = |label: &str, value: &Option<String>| {
                        format!("{}={}", label, if value.is_some() { "set" } else { "unset" })
                    };
                    tracing::warn!(
                        target: "trace_commons::passkey",
                        "partial WebAuthn relying-party config detected ({}, {}, {}); \
                         passkey surface DISABLED (fail-closed) — all three of \
                         TRACE_COMMONS_WEBAUTHN_RP_ID/ORIGIN/NAME must be set together",
                        name_state("TRACE_COMMONS_WEBAUTHN_RP_ID", &rp_id),
                        name_state("TRACE_COMMONS_WEBAUTHN_RP_ORIGIN", &rp_origin),
                        name_state("TRACE_COMMONS_WEBAUTHN_RP_NAME", &rp_name),
                    );
                }
                None
            }
        }
    }

    fn non_blank_env(key: &str) -> Option<String> {
        std::env::var(key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }
}
