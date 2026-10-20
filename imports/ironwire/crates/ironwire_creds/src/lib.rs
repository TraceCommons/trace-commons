//! Credential discovery for IronWire backends.
//!
//! IronWire reads credentials the official clients already store on this
//! machine. It never mints them, never uploads them, and never attaches one to
//! a host other than its own issuer (`docs/TRUST.md` I2).
//!
//! A user may hold more than one login per product — Claude Code under several
//! `CLAUDE_CONFIG_DIR`s, Codex under several `CODEX_HOME`s. Each is located by
//! its directory ([`ClaudeLogin`], [`CodexLogin`]), described the same way to
//! every surface ([`SubscriptionLogin`]), and consented to on its own.
//!
//! Reuse note: `ironclaw_llm::auth` already parses Codex CLI credentials via
//! `CredentialSource::CodexCli`, and [`codex`] delegates to it — a plain
//! dependency, not behind any feature. The Claude Code reader is currently a
//! private function inside `ironclaw_llm::anthropic_oauth`, so it is ported
//! here; the upstream fix is to add `CredentialSource::ClaudeCode`
//! (`docs/DESIGN.md` §7).
#![warn(missing_docs)]

pub mod claude;
pub mod codex;
pub mod consent;
pub mod login;

use secrecy::SecretString;

pub use claude::{ClaudeCodeCredentials, ClaudeLogin};
pub use codex::{CodexCredentials, CodexLogin};
pub use consent::{ConsentLedger, ConsentPrompt, ConsentRecord};
pub use login::{SubscriptionLogin, SubscriptionProduct};

/// Errors from reading or refreshing a credential.
#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    /// No credential was found in any known location.
    #[error("{product} is not logged in on this machine (looked in {locations})")]
    NotFound {
        /// Human-readable product name, e.g. "Claude Code".
        product: &'static str,
        /// Where we looked, joined for display.
        locations: String,
    },

    /// One particular login holds no credential.
    ///
    /// Separate from [`Self::NotFound`] because with more than one login per
    /// product, "not logged in" has to say *which*, and the command that fixes
    /// it differs per login — so both travel with the error instead of each
    /// caller reconstructing them.
    #[error(
        "{product} is not logged in {} (looked in {looked}); run `{command}` to log in{}",
        on_or_for(.location.as_deref()),
        note_suffix(.note.as_deref())
    )]
    NotLoggedIn {
        /// Human-readable product name, e.g. "Claude Code".
        product: &'static str,
        /// The login's directory, home-abbreviated. `None` for a default login.
        location: Option<String>,
        /// Where we looked, joined for display.
        looked: String,
        /// What the user should run to log this login in.
        command: String,
        /// A likely cause worth naming, when there is one.
        note: Option<String>,
    },

    /// A credential was found but could not be parsed.
    #[error(
        "{product} credentials at {path} are not in a shape we recognise{}",
        log_in_again(.command.as_deref())
    )]
    Malformed {
        /// Human-readable product name.
        product: &'static str,
        /// Path that failed.
        path: String,
        /// What logs this login in again, when the reader knows which login
        /// it was reading.
        command: Option<String>,
    },

    /// The credential is present but expired and cannot be refreshed here.
    #[error("{product} credentials have expired; re-authenticate with `{command}`")]
    Expired {
        /// Human-readable product name.
        product: &'static str,
        /// What the user should run. Owned, because it names the login's
        /// directory when it is not the default one.
        command: String,
    },

    /// I/O failure.
    #[error("reading {product} credentials at {path}: {source}")]
    Io {
        /// Human-readable product name.
        product: &'static str,
        /// What we were reading.
        path: String,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
}

fn on_or_for(location: Option<&str>) -> String {
    match location {
        None => "on this machine".to_string(),
        Some(location) => format!("for {location}"),
    }
}

fn note_suffix(note: Option<&str>) -> String {
    note.map(|note| format!(". {note}")).unwrap_or_default()
}

fn log_in_again(command: Option<&str>) -> String {
    command
        .map(|command| format!("; run `{command}` to log in again"))
        .unwrap_or_default()
}

/// A bearer credential plus the single host it may be sent to.
///
/// The `issuer_host` field is not decoration: it is the enforcement point for
/// `docs/TRUST.md` I2. Upstream clients assert it before attaching the token.
#[derive(Clone)]
pub struct Bearer {
    /// The token. Never logged, never serialized.
    pub token: SecretString,
    /// The only host this token may be sent to.
    pub issuer_host: &'static str,
}

impl std::fmt::Debug for Bearer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bearer")
            .field("token", &"<redacted>")
            .field("issuer_host", &self.issuer_host)
            .finish()
    }
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, CredentialError>;
