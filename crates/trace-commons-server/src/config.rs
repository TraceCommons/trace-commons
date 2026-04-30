//! Minimal TraceCommons server database configuration.

use std::path::PathBuf;

use secrecy::{ExposeSecret, SecretString};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DatabaseBackend {
    #[default]
    Postgres,
    LibSql,
}

impl std::fmt::Display for DatabaseBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Postgres => write!(f, "postgres"),
            Self::LibSql => write!(f, "libsql"),
        }
    }
}

impl std::str::FromStr for DatabaseBackend {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "postgres" | "postgresql" | "pg" => Ok(Self::Postgres),
            "libsql" | "turso" | "sqlite" => Ok(Self::LibSql),
            _ => Err(format!(
                "invalid database backend '{}', expected 'postgres' or 'libsql'",
                s
            )),
        }
    }
}

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
    pub backend: DatabaseBackend,
    pub url: SecretString,
    pub pool_size: usize,
    pub ssl_mode: SslMode,
    pub libsql_path: Option<PathBuf>,
    pub libsql_url: Option<String>,
    pub libsql_auth_token: Option<SecretString>,
}

impl DatabaseConfig {
    pub fn from_postgres_url(url: &str, pool_size: usize) -> Self {
        Self {
            backend: DatabaseBackend::Postgres,
            url: SecretString::from(url.to_string()),
            pool_size,
            ssl_mode: SslMode::from_env(),
            libsql_path: None,
            libsql_url: None,
            libsql_auth_token: None,
        }
    }

    pub fn from_libsql_path(
        path: &str,
        turso_url: Option<&str>,
        turso_token: Option<&str>,
    ) -> Self {
        let turso_url = turso_url.filter(|s| !s.is_empty());
        let turso_token = turso_token.filter(|s| !s.is_empty());
        Self {
            backend: DatabaseBackend::LibSql,
            url: SecretString::from("unused://libsql".to_string()),
            pool_size: 1,
            ssl_mode: SslMode::default(),
            libsql_path: Some(PathBuf::from(path)),
            libsql_url: turso_url.map(String::from),
            libsql_auth_token: turso_token.map(|t| SecretString::from(t.to_string())),
        }
    }

    pub fn url(&self) -> &str {
        self.url.expose_secret()
    }
}

pub fn trace_commons_base_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".trace-commons")
}

pub fn default_libsql_path() -> PathBuf {
    trace_commons_base_dir().join("trace-commons.db")
}
