//! Codex CLI credentials — delegated to `ironclaw_llm`.
//!
//! This used to be a hand-rolled reader for `~/.codex/auth.json`. It is not any
//! more: `ironclaw_llm::auth` already does exactly this, in production, with
//! the auth-mode catalog and the ChatGPT-vs-API-key base-URL split handled. A
//! second implementation of credential discovery is a second thing to keep
//! correct as Codex changes its file format, and the failure mode is a user
//! silently losing their subscription.
//!
//! What stays here is IronWire's own shape: a [`Bearer`] bound to the one host
//! that issued it (`docs/TRUST.md` I2), which is a proxy concern `ironclaw_llm`
//! has no reason to model.

use std::path::{Path, PathBuf};

use ironclaw_llm::auth::{self, CredentialSource};
use secrecy::{ExposeSecret, SecretString};

use crate::login::{
    SubscriptionLogin, SubscriptionProduct, abbreviate_home, expand_home, shell_word,
};
use crate::{Bearer, CredentialError, Result};

/// Host serving the ChatGPT subscription backend.
pub const CHATGPT_HOST: &str = "chatgpt.com";
/// Base URL for the ChatGPT subscription backend.
pub const CHATGPT_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";

/// Host serving the metered OpenAI API.
pub const OPENAI_HOST: &str = "api.openai.com";
/// Base URL for the metered OpenAI API.
pub const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

/// Which capacity pool a Codex credential draws on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexMode {
    /// ChatGPT subscription: marginal cost zero, capacity scarce, consent
    /// required (`docs/TRUST.md` §2).
    ChatGpt,
    /// Metered OpenAI API key.
    ApiKey,
}

/// A discovered Codex credential.
#[derive(Clone)]
pub struct CodexCredentials {
    token: SecretString,
    /// Which pool this draws on.
    pub mode: CodexMode,
    /// Base URL the issuing tool recorded for this credential.
    pub base_url: String,
    /// Where this came from, for `ironwire doctor`.
    pub source: String,
    /// Whether a refresh token is available. Display only — IronWire does not
    /// drive a refresh itself (see below).
    pub refreshable: bool,
}

impl std::fmt::Debug for CodexCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexCredentials")
            .field("token", &"<redacted>")
            .field("mode", &self.mode)
            .field("base_url", &self.base_url)
            .field("source", &self.source)
            .finish()
    }
}

impl CodexCredentials {
    /// Base URL this credential is valid against.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The ChatGPT account this credential belongs to, read from its own JWT.
    ///
    /// The ChatGPT backend requires a `chatgpt-account-id` header, and Codex
    /// only sends one on its own built-in provider path — a request routed
    /// through a custom provider arrives without it. Deriving it from the token
    /// we are about to present is not identity forgery (`docs/TRUST.md` §3): it
    /// names the account that already owns the credential, and a value read from
    /// the token cannot address any other account.
    ///
    /// `None` for an API key, which is not a JWT and needs no account header.
    #[must_use]
    pub fn account_id(&self) -> Option<String> {
        if self.mode != CodexMode::ChatGpt {
            return None;
        }
        account_id_from_jwt(self.token.expose_secret())
    }

    /// When this credential expires, if it says.
    ///
    /// Read from the token's own `exp` claim. `None` for an API key, which does
    /// not expire, and for a token we cannot parse.
    #[must_use]
    pub fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        if self.mode != CodexMode::ChatGpt {
            return None;
        }
        expiry_from_jwt(self.token.expose_secret())
    }

    /// Whether the stored token has already expired.
    ///
    /// **IronWire does not refresh it.** `ironclaw_llm` can — its
    /// `refresh_access_token` posts to OpenAI's token endpoint and writes the
    /// result back to `auth.json` — and IronWire deliberately does not, for a
    /// reason worth stating: `~/.codex/auth.json` is Codex's file, not ours. A
    /// second writer racing Codex's own refresh can leave a user logged out of
    /// the tool they actually paid for, and the failure would look like Codex's
    /// bug rather than ours.
    ///
    /// So IronWire re-reads the file on every request, picks up whatever Codex
    /// wrote, and when the token really is stale says so in one clear sentence
    /// naming that login's `codex login` ([`CodexLogin::login_command`]). A user
    /// logging in again is a smaller cost than a proxy corrupting their login.
    #[must_use]
    pub fn is_expired(&self, now: chrono::DateTime<chrono::Utc>) -> bool {
        // A minute of slack: a token expiring while the request is in flight is
        // effectively expired, and reporting it early beats a 401 the user has
        // to interpret.
        self.expires_at()
            .is_some_and(|at| at <= now + chrono::Duration::seconds(60))
    }

    /// Bearer bound to the host matching this credential's mode.
    #[must_use]
    pub fn bearer(&self) -> Bearer {
        Bearer {
            token: self.token.clone(),
            issuer_host: match self.mode {
                CodexMode::ChatGpt => CHATGPT_HOST,
                CodexMode::ApiKey => OPENAI_HOST,
            },
        }
    }

    /// Read the credential Codex stored for its default home.
    ///
    /// # Errors
    ///
    /// [`CredentialError::NotFound`] when Codex is not logged in here.
    pub fn discover() -> Result<Self> {
        Self::from_path(None)
    }

    /// Read an explicit `auth.json`, or the default home's for `None`.
    ///
    /// This is how every Codex login is read — [`CodexLogin::read`] calls it
    /// with that login's file — so the parsing stays `ironclaw`'s in one place.
    /// It cannot say *which* login it was reading; prefer
    /// [`CodexLogin::read`], whose errors name the home and the command.
    ///
    /// # Errors
    ///
    /// [`CredentialError::NotFound`] when the file is absent or unusable.
    /// `ironclaw` does not distinguish the two, so neither can this.
    pub fn from_path(path: Option<PathBuf>) -> Result<Self> {
        let path = path.unwrap_or_else(auth_path);
        let persisted = auth::load_persisted_credentials(CredentialSource::CodexCli, Some(&path))
            .ok_or_else(|| CredentialError::NotFound {
            product: "Codex",
            locations: path.display().to_string(),
        })?;

        Ok(Self {
            token: persisted.token,
            // `is_subscription` is ironclaw's own reading of `auth_mode` plus
            // which credential is actually present — the distinction we need.
            mode: if persisted.is_subscription {
                CodexMode::ChatGpt
            } else {
                CodexMode::ApiKey
            },
            base_url: persisted.base_url,
            source: persisted.source_path.unwrap_or(path).display().to_string(),
            refreshable: persisted.refresh_token.is_some(),
        })
    }
}

/// Path Codex writes its credentials to.
///
/// `CODEX_HOME` is honoured here because Codex honours it — `ironclaw`'s default
/// path does not, and inheriting that would mean a user who moved their Codex
/// home is told they are not logged in when they are. The *parsing* still
/// belongs to `ironclaw`; only the location is ours.
#[must_use]
pub fn auth_path() -> PathBuf {
    if let Ok(home) = std::env::var("CODEX_HOME")
        && !home.is_empty()
    {
        return PathBuf::from(home).join("auth.json");
    }
    auth::default_credentials_path(CredentialSource::CodexCli)
}

/// One Codex login, located by the `CODEX_HOME` it was made under.
///
/// The default login is today's [`auth_path`] — which, unlike
/// [`crate::ClaudeLogin`]'s default, does follow `CODEX_HOME` in this
/// process's environment. That is kept for compatibility: hermetic tests and
/// existing setups plant `CODEX_HOME` precisely to move the default login.
///
/// A home whose `config.toml` sets `cli_auth_credentials_store` to `keyring`
/// or `auto` keeps its login in the system keyring rather than `auth.json`.
/// IronWire does not read that store; the not-logged-in error says so.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CodexLogin {
    /// What Codex saw as `CODEX_HOME`, `~` expanded. `None` for the default.
    codex_home: Option<PathBuf>,
    /// The user's home, for writing paths back as `~/…`.
    home: Option<PathBuf>,
}

impl Default for CodexLogin {
    fn default() -> Self {
        Self::default_login()
    }
}

/// Why a home can look logged out when it is not.
const KEYRING_NOTE: &str = "If the config.toml beside that auth.json sets \
     `cli_auth_credentials_store` to `keyring` or `auto`, Codex keeps the login in the \
     system keyring instead, which IronWire does not read; set \
     `cli_auth_credentials_store = \"file\"` there and log in again";

impl CodexLogin {
    /// The login Codex uses for its default home.
    #[must_use]
    pub fn default_login() -> Self {
        Self::with_home(None, dirs::home_dir())
    }

    /// The login Codex makes when run with `CODEX_HOME` set to `codex_home`.
    /// A leading `~` is expanded against the user's home.
    #[must_use]
    pub fn in_home(codex_home: impl Into<PathBuf>) -> Self {
        Self::with_home(Some(codex_home.into()), dirs::home_dir())
    }

    /// As [`Self::in_home`] (or [`Self::default_login`] for `None`), against an
    /// explicit home directory.
    #[must_use]
    pub fn with_home(codex_home: Option<PathBuf>, home: Option<PathBuf>) -> Self {
        let codex_home = codex_home
            .filter(|dir| !dir.as_os_str().is_empty())
            .map(|dir| match dir.to_str() {
                Some(raw) => PathBuf::from(expand_home(raw, home.as_deref())),
                None => dir,
            });
        Self { codex_home, home }
    }

    /// The `CODEX_HOME`, `~` expanded. `None` for the default login.
    #[must_use]
    pub fn codex_home(&self) -> Option<&Path> {
        self.codex_home.as_deref()
    }

    /// Whether this is the default login.
    #[must_use]
    pub fn is_default(&self) -> bool {
        self.codex_home.is_none()
    }

    /// The `auth.json` this login is stored in.
    #[must_use]
    pub fn auth_path(&self) -> PathBuf {
        match &self.codex_home {
            Some(dir) => dir.join("auth.json"),
            None => auth_path(),
        }
    }

    /// Read this login's credential.
    ///
    /// # Errors
    ///
    /// [`CredentialError::NotLoggedIn`] naming the home and the command to run
    /// when it holds no usable `auth.json`.
    pub fn read(&self) -> Result<CodexCredentials> {
        CodexCredentials::from_path(Some(self.auth_path())).map_err(|err| match err {
            CredentialError::NotFound { product, locations } => CredentialError::NotLoggedIn {
                product,
                location: self.location(),
                looked: locations,
                command: self.login_command(),
                note: Some(self.why_not_logged_in()),
            },
            other => other,
        })
    }

    /// The likeliest reason this home has no `auth.json`, beyond never having
    /// logged in.
    fn why_not_logged_in(&self) -> String {
        match (&self.codex_home, self.location()) {
            // Codex refuses a `CODEX_HOME` that does not exist, so the login
            // command alone would fail.
            (Some(dir), Some(location)) if !dir.exists() => format!(
                "Codex refuses a CODEX_HOME that does not exist; create it first with \
                 `mkdir -p {}`",
                shell_word(&location)
            ),
            _ => KEYRING_NOTE.to_string(),
        }
    }

    /// What to run to log this login in: `codex login`, or
    /// `CODEX_HOME=~/.codex-work codex login`.
    #[must_use]
    pub fn login_command(&self) -> String {
        match self.location() {
            None => "codex login".to_string(),
            Some(location) => format!("CODEX_HOME={} codex login", shell_word(&location)),
        }
    }

    /// Where this login lives, for people: `~/.codex`, `~/.codex-work`.
    #[must_use]
    pub fn display_location(&self) -> String {
        self.location().unwrap_or_else(|| {
            let path = auth_path();
            let dir = path.parent().unwrap_or(&path);
            abbreviate_home(&dir.display().to_string(), self.home.as_deref())
        })
    }

    /// This login as the control API, CLI and consent prompt name it.
    /// `default` says whether it is the backend `codex-sub`, which a
    /// relocated default login is despite having a home of its own.
    #[must_use]
    pub fn describe(&self, default: bool) -> SubscriptionLogin {
        SubscriptionLogin {
            product: SubscriptionProduct::ChatGpt,
            location: self.location(),
            config_dir: self.consent_location(),
            source_dir: self
                .auth_path()
                .parent()
                .map(|dir| dir.display().to_string()),
            default,
        }
    }

    /// Which login a consent to this one answered:
    /// [`SubscriptionLogin::consent_location`], without describing it first.
    ///
    /// `None` for the default login even when a `CODEX_HOME` in this process
    /// moves it ([`auth_path`]): that is the compatibility path hermetic
    /// setups rely on, and the record it matches is the one they wrote.
    #[must_use]
    pub fn consent_location(&self) -> Option<String> {
        self.codex_home
            .as_deref()
            .map(|dir| dir.to_string_lossy().into_owned())
    }

    /// The `CODEX_HOME`, home-abbreviated. `None` for the default login.
    fn location(&self) -> Option<String> {
        self.codex_home
            .as_deref()
            .map(|dir| abbreviate_home(&dir.display().to_string(), self.home.as_deref()))
    }
}

/// Read `chatgpt_account_id` out of a Codex access token.
///
/// The claim lives under the namespaced `https://api.openai.com/auth` object.
/// The signature is neither checked nor needed here — we are reading a claim out
/// of a token we already hold and are about to send to the issuer, which will
/// verify it. A token we cannot parse yields `None` and the header is simply
/// omitted, which is the same position we would be in without this function.
fn account_id_from_jwt(token: &str) -> Option<String> {
    jwt_claims(token)?
        .pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

/// Read the `exp` claim, so an expired login is reported as such rather than as
/// a bare 401.
fn expiry_from_jwt(token: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let exp = jwt_claims(token)?.get("exp")?.as_i64()?;
    chrono::DateTime::from_timestamp(exp, 0)
}

/// Decode a JWT payload.
///
/// The signature is neither checked nor needed: we are reading claims out of a
/// token we already hold and are about to present to its issuer, which will
/// verify it. A token we cannot parse yields `None`, which every caller treats
/// as "we do not know" rather than as a failure.
fn jwt_claims(token: &str) -> Option<serde_json::Value> {
    use base64::Engine as _;

    let payload = token.split('.').nth(1)?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    serde_json::from_slice(&decoded).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_auth(contents: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("auth.json");
        let mut file = std::fs::File::create(&path).expect("create");
        file.write_all(contents.as_bytes()).expect("write");
        (dir, path)
    }

    #[test]
    fn a_chatgpt_login_is_recognised_as_subscription_capacity() {
        let (_dir, path) = write_auth(
            r#"{
                "auth_mode": "chatgpt",
                "OPENAI_API_KEY": null,
                "tokens": {
                    "id_token": "eyJhbGciOi",
                    "access_token": "eyJACCESS",
                    "refresh_token": "rt.1.EXAMPLE",
                    "account_id": "36afe797-0000"
                },
                "last_refresh": "2026-07-30T04:20:00Z"
            }"#,
        );
        let creds = CodexCredentials::from_path(Some(path)).expect("reads");
        assert_eq!(creds.mode, CodexMode::ChatGpt);
        assert_eq!(creds.bearer().issuer_host, CHATGPT_HOST);
        assert!(creds.base_url().contains("chatgpt.com"));
        assert!(creds.refreshable);
    }

    #[test]
    fn an_api_key_login_routes_to_the_metered_host() {
        let (_dir, path) =
            write_auth(r#"{"auth_mode": "apiKey", "OPENAI_API_KEY": "sk-proj-EXAMPLE"}"#);
        let creds = CodexCredentials::from_path(Some(path)).expect("reads");
        assert_eq!(creds.mode, CodexMode::ApiKey);
        assert_eq!(creds.bearer().issuer_host, OPENAI_HOST);
        assert!(creds.base_url().contains("api.openai.com"));
    }

    #[test]
    fn a_missing_login_is_reported_as_not_found() {
        let err = CodexCredentials::from_path(Some(PathBuf::from("/nonexistent/auth.json")))
            .expect_err("must not invent a credential");
        assert!(matches!(err, CredentialError::NotFound { .. }));
    }

    /// A Codex-shaped access token. Header and signature are inert — only the
    /// payload is ever read.
    fn jwt_with(payload: &serde_json::Value) -> String {
        use base64::Engine as _;
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        format!(
            "{}.{}.{}",
            engine.encode(br#"{"alg":"RS256","typ":"JWT"}"#),
            engine.encode(payload.to_string()),
            engine.encode(b"not-a-real-signature"),
        )
    }

    #[test]
    fn the_account_id_comes_out_of_the_token_we_are_about_to_present() {
        // TRUST.md §3: the header names the account that owns the credential.
        // There is no path here that could address a different account.
        let token = jwt_with(&serde_json::json!({
            "https://api.openai.com/auth": {"chatgpt_account_id": "36afe797-0000"},
            "exp": 1_800_000_000u64,
        }));
        let (_dir, path) = write_auth(&format!(
            r#"{{"auth_mode": "chatgpt", "tokens": {{"access_token": "{token}", "refresh_token": "rt"}}}}"#
        ));
        let creds = CodexCredentials::from_path(Some(path)).expect("reads");
        assert_eq!(creds.account_id().as_deref(), Some("36afe797-0000"));
    }

    #[test]
    fn an_api_key_has_no_account_id_to_report() {
        let (_dir, path) =
            write_auth(r#"{"auth_mode": "apiKey", "OPENAI_API_KEY": "sk-proj-EXAMPLE"}"#);
        let creds = CodexCredentials::from_path(Some(path)).expect("reads");
        assert_eq!(creds.account_id(), None);
    }

    #[test]
    fn a_token_we_cannot_parse_omits_the_header_rather_than_guessing() {
        assert_eq!(account_id_from_jwt("not-a-jwt"), None);
        assert_eq!(account_id_from_jwt("a.!!!not-base64!!!.c"), None);
        // Well-formed, but the claim is simply not there.
        assert_eq!(
            account_id_from_jwt(&jwt_with(&serde_json::json!({"sub": "user_1"}))),
            None
        );
    }

    #[test]
    fn debug_never_leaks_the_token() {
        let (_dir, path) =
            write_auth(r#"{"auth_mode": "apiKey", "OPENAI_API_KEY": "sk-proj-SECRETVALUE"}"#);
        let creds = CodexCredentials::from_path(Some(path)).expect("reads");
        assert!(!format!("{creds:?}").contains("SECRETVALUE"));
        assert!(!format!("{:?}", creds.bearer()).contains("SECRETVALUE"));
    }
}

#[cfg(test)]
mod expiry_tests {
    use super::*;
    use std::io::Write;

    fn write_auth(contents: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("auth.json");
        let mut file = std::fs::File::create(&path).expect("create");
        file.write_all(contents.as_bytes()).expect("write");
        (dir, path)
    }

    fn token_expiring_at(exp: i64) -> String {
        use base64::Engine as _;
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let payload = serde_json::json!({
            "https://api.openai.com/auth": {"chatgpt_account_id": "acct"},
            "exp": exp,
        });
        format!(
            "{}.{}.{}",
            engine.encode(br#"{"alg":"RS256"}"#),
            engine.encode(payload.to_string()),
            engine.encode(b"inert"),
        )
    }

    fn creds_expiring_at(exp: i64) -> (tempfile::TempDir, CodexCredentials) {
        let (dir, path) = write_auth(&format!(
            r#"{{"auth_mode":"chatgpt","tokens":{{"access_token":"{}","refresh_token":"rt"}}}}"#,
            token_expiring_at(exp)
        ));
        let creds = CodexCredentials::from_path(Some(path)).expect("reads");
        (dir, creds)
    }

    #[test]
    fn a_live_token_is_not_expired() {
        let now = chrono::Utc::now();
        let (_dir, creds) = creds_expiring_at(now.timestamp() + 3600);
        assert!(!creds.is_expired(now));
        assert!(creds.expires_at().is_some());
    }

    #[test]
    fn a_stale_token_is_reported_rather_than_presented() {
        // Presenting it produces a 401 the user has to interpret. Saying
        // "expired, run `codex`" is one sentence and one command.
        let now = chrono::Utc::now();
        let (_dir, creds) = creds_expiring_at(now.timestamp() - 60);
        assert!(creds.is_expired(now));
    }

    #[test]
    fn a_token_expiring_within_the_minute_counts_as_expired() {
        // It would expire mid-request otherwise, and a 401 halfway through is
        // worse than a clear refusal up front.
        let now = chrono::Utc::now();
        let (_dir, creds) = creds_expiring_at(now.timestamp() + 10);
        assert!(creds.is_expired(now));
    }

    #[test]
    fn a_token_with_no_expiry_claim_is_never_assumed_stale() {
        // We do not know, and guessing "expired" would lock a working user out
        // of their own subscription.
        let (_dir, path) = write_auth(
            r#"{"auth_mode":"chatgpt","tokens":{"access_token":"not-a-jwt","refresh_token":"rt"}}"#,
        );
        let creds = CodexCredentials::from_path(Some(path)).expect("reads");
        assert_eq!(creds.expires_at(), None);
        assert!(!creds.is_expired(chrono::Utc::now()));
    }

    #[test]
    fn an_api_key_never_expires() {
        let (_dir, path) = write_auth(r#"{"auth_mode":"apiKey","OPENAI_API_KEY":"sk-proj-x"}"#);
        let creds = CodexCredentials::from_path(Some(path)).expect("reads");
        assert_eq!(creds.expires_at(), None);
        assert!(!creds.is_expired(chrono::Utc::now()));
    }
}

#[cfg(test)]
mod login_tests {
    use super::*;

    const CHATGPT_AUTH: &str =
        r#"{"auth_mode":"chatgpt","tokens":{"access_token":"not-a-jwt","refresh_token":"rt"}}"#;

    fn alice(codex_home: &str) -> CodexLogin {
        CodexLogin::with_home(Some(codex_home.into()), Some("/Users/alice".into()))
    }

    #[test]
    fn an_additional_codex_home_reads_its_own_auth_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("auth.json");
        std::fs::write(&file, CHATGPT_AUTH).expect("write");

        let login = CodexLogin::with_home(Some(dir.path().to_path_buf()), None);
        assert_eq!(login.auth_path(), file);
        let creds = login.read().expect("reads its own home");
        assert_eq!(creds.mode, CodexMode::ChatGpt);
        assert_eq!(creds.source, file.display().to_string());
    }

    #[test]
    fn a_codex_home_with_no_login_names_the_home_the_command_and_keyring_mode() {
        let home = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(home.path().join(".codex-work")).expect("mkdir");
        let login = CodexLogin::with_home(
            Some(home.path().join(".codex-work")),
            Some(home.path().to_path_buf()),
        );
        let err = login.read().expect_err("nothing there");
        assert!(matches!(err, CredentialError::NotLoggedIn { .. }));
        let message = err.to_string();
        assert!(message.contains("for ~/.codex-work"), "{message}");
        assert!(
            message.contains("`CODEX_HOME=~/.codex-work codex login`"),
            "{message}"
        );
        assert!(
            message.contains(r#"cli_auth_credentials_store = "file""#),
            "{message}"
        );
    }

    #[test]
    fn a_codex_home_that_does_not_exist_says_to_create_it_before_logging_in() {
        let home = tempfile::tempdir().expect("tempdir");
        let login = CodexLogin::with_home(
            Some(home.path().join(".codex-work")),
            Some(home.path().to_path_buf()),
        );
        let message = login.read().expect_err("nothing there").to_string();
        assert!(message.contains("`mkdir -p ~/.codex-work`"), "{message}");
        assert!(
            message.contains("`CODEX_HOME=~/.codex-work codex login`"),
            "{message}"
        );
    }

    #[test]
    fn the_default_codex_login_is_still_todays_auth_path() {
        // Hermetic tests plant CODEX_HOME to move the default login; that has
        // to keep working.
        assert_eq!(CodexLogin::default_login().auth_path(), auth_path());
        assert!(CodexLogin::default_login().is_default());
    }

    #[test]
    fn a_codex_login_command_names_its_home() {
        assert_eq!(
            alice("/Users/alice/.codex-work").login_command(),
            "CODEX_HOME=~/.codex-work codex login"
        );
        assert_eq!(
            alice("~/.codex-work").codex_home(),
            Some(Path::new("/Users/alice/.codex-work"))
        );
        assert_eq!(
            CodexLogin::with_home(None, Some("/Users/alice".into())).login_command(),
            "codex login"
        );
    }

    #[test]
    fn a_codex_login_is_described_by_its_home() {
        let work = alice("~/.codex-work");
        assert_eq!(work.display_location(), "~/.codex-work");
        let described = work.describe(false);
        assert_eq!(described.label(), "ChatGPT subscription (~/.codex-work)");
        assert_eq!(
            described.connect_command(),
            "ironwire connect codex --config-dir ~/.codex-work"
        );
        assert_eq!(
            CodexLogin::default_login().describe(true),
            SubscriptionLogin::default_for(SubscriptionProduct::ChatGpt)
        );
    }

    #[test]
    fn a_codex_login_is_consented_to_by_its_expanded_home() {
        let moved_default = alice("~/.codex-main").describe(true);
        assert_eq!(moved_default.location.as_deref(), Some("~/.codex-main"));
        assert_eq!(
            moved_default.consent_location(),
            Some("/Users/alice/.codex-main"),
            "a relocated codex-sub is consented to by its home"
        );
        assert_eq!(
            alice("~/.codex-main").consent_location().as_deref(),
            Some("/Users/alice/.codex-main")
        );
        assert_eq!(CodexLogin::default_login().consent_location(), None);
    }
}
