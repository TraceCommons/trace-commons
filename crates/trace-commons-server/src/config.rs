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
}

impl DatabaseConfig {
    pub fn from_postgres_url(url: &str, pool_size: usize) -> Self {
        Self {
            url: SecretString::from(url.to_string()),
            pool_size,
            ssl_mode: SslMode::from_env(),
        }
    }

    pub fn url(&self) -> &str {
        self.url.expose_secret()
    }
}
