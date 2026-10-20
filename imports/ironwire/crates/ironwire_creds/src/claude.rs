//! Claude Code subscription credentials.
//!
//! Claude Code keeps one OAuth login per config dir. With `CLAUDE_CONFIG_DIR`
//! unset it uses the macOS Keychain item `Claude Code-credentials` and, where
//! there is no Keychain, `~/.claude/.credentials.json`. With it set, both move:
//! the file to `<dir>/.credentials.json`, and the Keychain item to a name
//! derived from the dir ([`ClaudeLogin::keychain_services`]). The token
//! authenticates against `api.anthropic.com` with `Authorization: Bearer` plus
//! the `anthropic-beta: oauth-2025-04-20` flag — `x-api-key` is rejected for it.
//!
//! Ported from `ironclaw_llm::anthropic_oauth`, which proves this path in
//! production but keeps the reader private.

use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeZone, Utc};
use secrecy::SecretString;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use crate::login::{
    SubscriptionLogin, SubscriptionProduct, abbreviate_home, expand_home, shell_word,
};
use crate::{Bearer, CredentialError, Result};

/// The only host a Claude subscription token may be sent to.
pub const ANTHROPIC_HOST: &str = "api.anthropic.com";

/// API version required alongside the OAuth beta flag. The newer dated
/// versions are not valid with it.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Beta flag that enables OAuth bearer auth. Without it the API answers 401
/// "OAuth authentication is currently not supported."
pub const ANTHROPIC_OAUTH_BETA: &str = "oauth-2025-04-20";

/// Access tokens Claude Code issues carry this prefix. Anything else in the
/// credential store is some other product's secret and is left alone.
const ACCESS_TOKEN_PREFIX: &str = "sk-ant-oat";

/// macOS Keychain service name of the default login.
const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

const PRODUCT: &str = "Claude Code";

/// One Claude Code login, located by the config dir it was made under.
///
/// The default login is the one Claude Code uses with `CLAUDE_CONFIG_DIR`
/// unset. It deliberately does *not* follow a `CLAUDE_CONFIG_DIR` in this
/// process's environment: the daemon runs under launchd or systemd with no
/// such variable, so honouring it would make the CLI and the daemon read two
/// different logins under one name. A CLI that sees the variable turns it into
/// an explicit config dir instead.
///
/// Two variables Claude Code also consults are ignored on purpose:
/// `CLAUDE_CODE_CUSTOM_OAUTH_URL` (a non-production OAuth server, which adds a
/// `-custom-oauth` infix to the Keychain name) and
/// `CLAUDE_SECURESTORAGE_CONFIG_DIR` (undocumented; it relocates the store
/// separately from the config dir). Neither describes a login a user made with
/// the shipped client.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClaudeLogin {
    /// Exactly what Claude Code saw as `CLAUDE_CONFIG_DIR`, after the shell
    /// expanded `~`. `None` for the default login.
    config_dir: Option<String>,
    /// The user's home, for the default login's file and for writing paths
    /// back as `~/…`.
    home: Option<PathBuf>,
    /// Whether [`Self::read`] may ask the macOS Keychain. Only
    /// [`Self::without_keychain`] turns it off.
    keychain: bool,
}

impl Default for ClaudeLogin {
    fn default() -> Self {
        Self::default_login()
    }
}

impl ClaudeLogin {
    /// The login Claude Code uses with `CLAUDE_CONFIG_DIR` unset.
    #[must_use]
    pub fn default_login() -> Self {
        Self::with_home(None, dirs::home_dir())
    }

    /// The login Claude Code makes when run with `CLAUDE_CONFIG_DIR` set to
    /// `config_dir`. A leading `~` is expanded against the user's home, as the
    /// shell would have done before Claude Code saw it.
    #[must_use]
    pub fn in_dir(config_dir: impl Into<String>) -> Self {
        Self::with_home(Some(config_dir.into()), dirs::home_dir())
    }

    /// As [`Self::in_dir`] (or [`Self::default_login`] for `None`), against an
    /// explicit home directory.
    ///
    /// An empty config dir is the default login, because that is how Claude
    /// Code treats an empty `CLAUDE_CONFIG_DIR`.
    #[must_use]
    pub fn with_home(config_dir: Option<String>, home: Option<PathBuf>) -> Self {
        let config_dir = config_dir
            .filter(|dir| !dir.is_empty())
            .map(|dir| expand_home(&dir, home.as_deref()));
        Self {
            config_dir,
            home,
            keychain: true,
        }
    }

    /// The same login, read from [`Self::credentials_path`] alone.
    ///
    /// For conformance tests outside this crate, which plant a credential file
    /// and must get the same answer on every machine. Asking the real Keychain
    /// from a test is asking whatever this developer's Keychain holds — and a
    /// locked one can stop a test run on a password prompt. Production never
    /// calls this: there, skipping the Keychain would read a file Claude Code
    /// is not using.
    #[must_use]
    pub fn without_keychain(mut self) -> Self {
        self.keychain = false;
        self
    }

    /// The config dir, `~` expanded. `None` for the default login.
    #[must_use]
    pub fn config_dir(&self) -> Option<&str> {
        self.config_dir.as_deref()
    }

    /// Whether this is the login Claude Code uses with no config dir.
    #[must_use]
    pub fn is_default(&self) -> bool {
        self.config_dir.is_none()
    }

    /// The macOS Keychain services this login may be stored under, in the
    /// order to try them.
    ///
    /// Claude Code names the item `Claude Code-credentials-` plus the first
    /// eight lowercase hex digits of sha256 over the NFC form of the exact
    /// `CLAUDE_CONFIG_DIR` string — no realpath, no trailing-slash trim. So
    /// `~/.claude-work` and `~/.claude-work/` are two different items, and an
    /// explicitly exported `~/.claude` is not the default login's item.
    /// A filesystem alias is not another place to look: it can hold another
    /// account, even though the two clients share a settings file.
    #[must_use]
    pub fn keychain_services(&self) -> Vec<String> {
        let Some(dir) = self.config_dir.as_deref() else {
            return vec![KEYCHAIN_SERVICE.to_string()];
        };
        vec![keychain_service_for(dir)]
    }

    /// The file Claude Code writes this login to where there is no Keychain,
    /// and on macOS when a Keychain write fails.
    #[must_use]
    pub fn credentials_path(&self) -> PathBuf {
        match &self.config_dir {
            Some(dir) => Path::new(dir).join(".credentials.json"),
            None => self
                .home
                .clone()
                .unwrap_or_default()
                .join(".claude")
                .join(".credentials.json"),
        }
    }

    /// Read this login's credential.
    ///
    /// Same order as Claude Code: the Keychain (on macOS), and the file only
    /// when no Keychain item exists. An item that exists but holds something
    /// unrecognisable is reported, not skipped — Claude Code would use it, so
    /// falling through to a stale file would read a login Claude Code is not
    /// using.
    ///
    /// # Errors
    ///
    /// [`CredentialError::NotLoggedIn`] when this login holds no credential,
    /// [`CredentialError::Malformed`] when the store holds something we do not
    /// recognise, [`CredentialError::Io`] when the Keychain cannot be checked
    /// or the file cannot be read. Only an absent Keychain item permits a file
    /// fallback; a locked or failed store may still contain the active login.
    pub fn read(&self) -> Result<ClaudeCodeCredentials> {
        if cfg!(target_os = "macos") && self.keychain {
            self.read_with(Some(&read_keychain))
        } else {
            self.read_with(None)
        }
    }

    /// [`Self::read`] with the Keychain lookup supplied, so tests never touch
    /// the real Keychain. `None` means there is no Keychain to ask.
    fn read_with(&self, keychain: Option<&KeychainLookup>) -> Result<ClaudeCodeCredentials> {
        let command = self.login_command();
        let mut looked = Vec::new();

        if let Some(keychain) = keychain {
            for service in self.keychain_services() {
                let label = format!("Keychain:{service}");
                let entry = keychain(&service).map_err(|_| CredentialError::Io {
                    product: PRODUCT,
                    path: label.clone(),
                    source: keychain_error(),
                })?;
                if let Some(json) = entry {
                    return ClaudeCodeCredentials::parse_for(&json, label, Some(command));
                }
                looked.push(label);
            }
        }

        let path = self.credentials_path();
        let shown = path.display().to_string();
        looked.push(shown.clone());
        match std::fs::read_to_string(&path) {
            Ok(json) => ClaudeCodeCredentials::parse_for(&json, shown, Some(command)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(CredentialError::NotLoggedIn {
                    product: PRODUCT,
                    location: self.location(),
                    looked: looked.join(", "),
                    command,
                    note: None,
                })
            }
            Err(source) => Err(CredentialError::Io {
                product: PRODUCT,
                path: shown,
                source,
            }),
        }
    }

    /// What to run to log this login in: `claude`, or
    /// `CLAUDE_CONFIG_DIR=~/.claude-work claude`.
    ///
    /// The `~/` form expands back to exactly the string this login hashes, so
    /// running it logs in the very Keychain item IronWire reads.
    #[must_use]
    pub fn login_command(&self) -> String {
        match self.location() {
            None => "claude".to_string(),
            Some(location) => format!("CLAUDE_CONFIG_DIR={} claude", shell_word(&location)),
        }
    }

    /// Where this login lives, for people: `~/.claude`, `~/.claude-work`.
    #[must_use]
    pub fn display_location(&self) -> String {
        self.location().unwrap_or_else(|| "~/.claude".to_string())
    }

    /// This login as the control API, CLI and consent prompt name it.
    /// `default` says whether it is the backend `claude-sub`, which a
    /// relocated default login is despite having a directory.
    #[must_use]
    pub fn describe(&self, default: bool) -> SubscriptionLogin {
        SubscriptionLogin {
            product: SubscriptionProduct::Claude,
            location: self.location(),
            config_dir: self.config_dir.clone(),
            source_dir: self
                .credentials_path()
                .parent()
                .map(|dir| dir.display().to_string()),
            default,
        }
    }

    /// Which login a consent to this one answered:
    /// [`SubscriptionLogin::consent_location`], without describing it first.
    #[must_use]
    pub fn consent_location(&self) -> Option<&str> {
        self.config_dir()
    }

    /// The config dir, home-abbreviated. `None` for the default login.
    fn location(&self) -> Option<String> {
        self.config_dir
            .as_deref()
            .map(|dir| abbreviate_home(dir, self.home.as_deref()))
    }
}

/// Fetch the secret stored under a Keychain service, if there is one.
type KeychainLookup = dyn Fn(&str) -> std::io::Result<Option<String>>;

/// The Keychain service Claude Code uses for `CLAUDE_CONFIG_DIR=dir`.
fn keychain_service_for(dir: &str) -> String {
    let normalized: String = dir.nfc().collect();
    let digest = Sha256::digest(normalized.as_bytes());
    let short: String = digest[..4].iter().map(|b| format!("{b:02x}")).collect();
    format!("{KEYCHAIN_SERVICE}-{short}")
}

#[derive(Debug, Deserialize)]
struct CredentialFile {
    #[serde(rename = "claudeAiOauth")]
    oauth: OauthBlock,
}

#[derive(Debug, Deserialize)]
struct OauthBlock {
    #[serde(rename = "accessToken")]
    access_token: String,
    #[serde(rename = "expiresAt")]
    expires_at: Option<i64>,
    #[serde(rename = "subscriptionType")]
    subscription_type: Option<String>,
    #[serde(rename = "rateLimitTier")]
    rate_limit_tier: Option<String>,
}

/// A discovered Claude Code subscription credential.
#[derive(Clone)]
pub struct ClaudeCodeCredentials {
    access_token: SecretString,
    /// When the token expires, if the store said.
    pub expires_at: Option<DateTime<Utc>>,
    /// Plan name as the store reports it, e.g. `max`. Display only.
    pub subscription_type: Option<String>,
    /// Rate-limit tier as the store reports it. Display only.
    pub rate_limit_tier: Option<String>,
    /// Where this came from, for `ironwire doctor`.
    pub source: String,
}

impl std::fmt::Debug for ClaudeCodeCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeCodeCredentials")
            .field("access_token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .field("subscription_type", &self.subscription_type)
            .field("source", &self.source)
            .finish()
    }
}

impl ClaudeCodeCredentials {
    /// Bearer bound to `api.anthropic.com`.
    #[must_use]
    pub fn bearer(&self) -> Bearer {
        Bearer {
            token: self.access_token.clone(),
            issuer_host: ANTHROPIC_HOST,
        }
    }

    /// Whether the token is past its stated expiry.
    ///
    /// A token with no stated expiry is treated as live: Claude Code refreshes
    /// in the background, so the honest move is to try it and let a 401 drive
    /// a re-read rather than refusing pre-emptively.
    #[must_use]
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|exp| now >= exp)
    }

    /// Read the default login's credential — [`ClaudeLogin::default_login`].
    ///
    /// # Errors
    ///
    /// As [`ClaudeLogin::read`].
    pub fn discover() -> Result<Self> {
        ClaudeLogin::default_login().read()
    }

    /// Parse a credential document. Public for the conformance tests.
    ///
    /// # Errors
    ///
    /// [`CredentialError::Malformed`] when the document does not hold a
    /// recognisable Claude Code access token.
    pub fn parse(json: &str, source: String) -> Result<Self> {
        Self::parse_for(json, source, None)
    }

    /// [`Self::parse`], with what to run should the document be unusable.
    fn parse_for(json: &str, source: String, command: Option<String>) -> Result<Self> {
        // A store entry whose token is not Claude Code's is some other
        // product's secret. Refusing it is how we avoid ever sending a
        // credential to a host that did not issue it.
        let Some(parsed) = serde_json::from_str::<CredentialFile>(json)
            .ok()
            .filter(|parsed| parsed.oauth.access_token.starts_with(ACCESS_TOKEN_PREFIX))
        else {
            return Err(CredentialError::Malformed {
                product: PRODUCT,
                path: source,
                command,
            });
        };

        Ok(Self {
            access_token: SecretString::from(parsed.oauth.access_token),
            expires_at: parsed.oauth.expires_at.and_then(millis_to_utc),
            subscription_type: parsed.oauth.subscription_type,
            rate_limit_tier: parsed.oauth.rate_limit_tier,
            source,
        })
    }
}

/// Path Claude Code writes the default login's credentials to on non-macOS
/// platforms. [`ClaudeLogin::credentials_path`] for any other login.
#[must_use]
pub fn credentials_path() -> PathBuf {
    ClaudeLogin::default_login().credentials_path()
}

fn read_keychain(service: &str) -> std::io::Result<Option<String>> {
    let mut command = std::process::Command::new("security");
    command.args(["find-generic-password", "-s", service, "-w"]);
    keychain_output(&mut command, std::time::Duration::from_secs(5))
}

/// A locked Keychain must not occupy a status worker forever. Bound both its
/// wait and its output; kill and reap on every unsuccessful wait path.
fn keychain_output(
    command: &mut std::process::Command,
    timeout: std::time::Duration,
) -> std::io::Result<Option<String>> {
    use std::io::Read;
    use std::process::Stdio;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| keychain_error())?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(keychain_error());
    };
    let reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        stdout
            .take(1024 * 1024 + 1)
            .read_to_end(&mut output)
            .map_err(|_| keychain_error())?;
        if output.len() > 1024 * 1024 {
            return Err(keychain_error());
        }
        Ok(output)
    });
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(keychain_error());
            }
        }
    };
    let output = reader.join().map_err(|_| keychain_error())??;
    let status = status?;
    // security returns errSecItemNotFound (-25300) modulo 256 as exit 44.
    // Every other failure may mean an existing active item is inaccessible.
    if status.code() == Some(44) {
        return Ok(None);
    }
    if !status.success() {
        return Err(keychain_error());
    }
    String::from_utf8(output)
        .map(|text| Some(text.trim().to_string()))
        .map_err(|_| keychain_error())
}

fn keychain_error() -> std::io::Error {
    // Never repeat subprocess output: it is credential data, including on an
    // unsuccessful command. The user needs the recovery action, not stderr.
    std::io::Error::other(
        "the Keychain lookup failed or timed out; unlock your macOS Keychain and \
         retry `ironwire doctor`; a credentials file was not used as a fallback",
    )
}

fn millis_to_utc(millis: i64) -> Option<DateTime<Utc>> {
    Utc.timestamp_millis_opt(millis).single()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "claudeAiOauth": {
            "accessToken": "sk-ant-oat01-EXAMPLE",
            "refreshToken": "sk-ant-ort01-EXAMPLE",
            "expiresAt": 1786165549135,
            "scopes": ["user:inference"],
            "subscriptionType": "max",
            "rateLimitTier": "default_claude_max"
        }
    }"#;

    #[test]
    fn parses_the_shape_claude_code_actually_writes() {
        let creds = ClaudeCodeCredentials::parse(SAMPLE, "test".into()).expect("parses");
        assert_eq!(creds.subscription_type.as_deref(), Some("max"));
        assert_eq!(creds.rate_limit_tier.as_deref(), Some("default_claude_max"));
        assert!(creds.expires_at.is_some());
    }

    #[test]
    fn a_token_is_bound_to_the_host_that_issued_it() {
        let creds = ClaudeCodeCredentials::parse(SAMPLE, "test".into()).expect("parses");
        assert_eq!(creds.bearer().issuer_host, ANTHROPIC_HOST);
    }

    #[test]
    fn a_foreign_token_in_the_store_is_refused() {
        // TRUST.md I2: we must never pick up someone else's secret and send it
        // to Anthropic just because it was in a file we read.
        let foreign = r#"{"claudeAiOauth": {"accessToken": "ghp_someothersecret"}}"#;
        let err = ClaudeCodeCredentials::parse(foreign, "test".into())
            .expect_err("must refuse a non-Claude token");
        assert!(matches!(err, CredentialError::Malformed { .. }));
    }

    #[test]
    fn garbage_is_malformed_not_a_panic() {
        assert!(ClaudeCodeCredentials::parse("not json", "test".into()).is_err());
        assert!(ClaudeCodeCredentials::parse("{}", "test".into()).is_err());
    }

    #[test]
    fn expiry_is_honoured_when_stated_and_ignored_when_absent() {
        let creds = ClaudeCodeCredentials::parse(SAMPLE, "test".into()).expect("parses");
        let expires = creds.expires_at.expect("sample states an expiry");
        assert!(creds.is_expired(expires));
        assert!(!creds.is_expired(expires - chrono::Duration::seconds(1)));

        let no_expiry = r#"{"claudeAiOauth": {"accessToken": "sk-ant-oat01-X"}}"#;
        let creds = ClaudeCodeCredentials::parse(no_expiry, "test".into()).expect("parses");
        assert!(
            !creds.is_expired(Utc::now()),
            "an unstated expiry must not pre-emptively disable the backend"
        );
    }

    #[test]
    fn debug_never_leaks_the_token() {
        let creds = ClaudeCodeCredentials::parse(SAMPLE, "test".into()).expect("parses");
        let rendered = format!("{creds:?}");
        assert!(!rendered.contains("EXAMPLE"), "token leaked into Debug");
        assert!(rendered.contains("redacted"));

        let rendered = format!("{:?}", creds.bearer());
        assert!(
            !rendered.contains("EXAMPLE"),
            "token leaked into Bearer Debug"
        );
    }
}

#[cfg(test)]
mod login_tests {
    use super::*;

    const SAMPLE: &str = r#"{"claudeAiOauth": {"accessToken": "sk-ant-oat01-EXAMPLE"}}"#;

    fn alice(config_dir: &str) -> ClaudeLogin {
        ClaudeLogin::with_home(Some(config_dir.into()), Some("/Users/alice".into()))
    }

    type Asked = std::rc::Rc<std::cell::RefCell<Vec<String>>>;

    /// A Keychain that holds exactly `items` and records every service asked
    /// for — the real one is never touched by a test.
    fn keychain(
        items: &[(&str, &str)],
    ) -> (
        impl Fn(&str) -> std::io::Result<Option<String>> + use<>,
        Asked,
    ) {
        let asked = Asked::default();
        let items: Vec<(String, String)> = items
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        let log = asked.clone();
        let lookup = move |service: &str| {
            log.borrow_mut().push(service.to_string());
            Ok(items
                .iter()
                .find(|(k, _)| k == service)
                .map(|(_, v)| v.clone()))
        };
        (lookup, asked)
    }

    // The three worked examples below were computed from Claude Code's own
    // derivation (2.1.42 and 2.1.286 agree). If Claude Code changes it, these
    // are the tests that should fail first.

    #[test]
    fn a_config_dir_keys_its_own_keychain_item_exactly_as_claude_code_does() {
        assert_eq!(
            alice("/Users/alice/.claude-work").keychain_services()[0],
            "Claude Code-credentials-be865d75"
        );
    }

    #[test]
    fn a_trailing_slash_is_part_of_what_claude_code_hashes() {
        assert_eq!(
            alice("/Users/alice/.claude-work/").keychain_services()[0],
            "Claude Code-credentials-95672096"
        );
    }

    #[test]
    fn an_explicitly_exported_default_dir_is_not_the_default_logins_keychain_item() {
        assert_eq!(
            alice("/Users/alice/.claude").keychain_services(),
            vec!["Claude Code-credentials-5ac5e874".to_string()]
        );
        assert_eq!(
            ClaudeLogin::with_home(None, Some("/Users/alice".into())).keychain_services(),
            vec!["Claude Code-credentials".to_string()]
        );
    }

    #[test]
    fn a_trailing_slash_dir_never_reads_another_logins_keychain_item() {
        let (lookup, asked) = keychain(&[("Claude Code-credentials-be865d75", SAMPLE)]);
        let result = alice("/Users/alice/.claude-work/").read_with(Some(&lookup));
        assert!(result.is_err(), "another login was accepted");
        assert_eq!(
            *asked.borrow(),
            vec!["Claude Code-credentials-95672096".to_string()]
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_hung_keychain_lookup_is_killed_and_reaped_within_its_deadline() {
        let mut command = std::process::Command::new("/bin/sleep");
        command.arg("10");
        let began = std::time::Instant::now();
        assert!(keychain_output(&mut command, std::time::Duration::from_millis(50)).is_err());
        assert!(began.elapsed() < std::time::Duration::from_secs(2));
        let mut command = std::process::Command::new("/bin/echo");
        command.arg("credential-output");
        assert_eq!(
            keychain_output(&mut command, std::time::Duration::from_secs(1))
                .expect("successful lookup")
                .as_deref(),
            Some("credential-output")
        );
    }

    #[test]
    #[cfg(unix)]
    fn only_an_absent_keychain_item_allows_a_file_fallback() {
        for (code, absent) in [(44, true), (1, false)] {
            let mut command = std::process::Command::new("/bin/sh");
            command.args(["-c", &format!("printf inert-keychain-secret; exit {code}")]);
            let result = keychain_output(&mut command, std::time::Duration::from_secs(1));
            if absent {
                assert!(matches!(result, Ok(None)));
            } else {
                let error = result.expect_err("failed lookup refuses");
                assert!(error.to_string().contains("unlock your macOS Keychain"));
                assert!(!error.to_string().contains("inert-keychain-secret"));
            }
        }
    }

    #[test]
    fn a_failed_keychain_lookup_never_uses_a_stale_file_login() {
        let dir = tempfile::tempdir().expect("login directory");
        std::fs::write(dir.path().join(".credentials.json"), SAMPLE).expect("stale login");
        let login = ClaudeLogin::with_home(Some(dir.path().display().to_string()), None);
        let unavailable = |_: &str| -> std::io::Result<Option<String>> {
            Err(std::io::Error::other("inert-secret-in-subprocess-error"))
        };
        let error = login
            .read_with(Some(&unavailable))
            .expect_err("Keychain refused");
        assert!(matches!(error, CredentialError::Io { .. }));
        assert!(error.to_string().contains("unlock your macOS Keychain"));
        assert!(!error.to_string().contains("inert-secret"));

        let absent = |_: &str| -> std::io::Result<Option<String>> { Ok(None) };
        let credential = login
            .read_with(Some(&absent))
            .expect("absent store permits file");
        assert_eq!(
            credential.source,
            login.credentials_path().display().to_string()
        );
    }

    #[test]
    fn the_keychain_hash_is_over_the_nfc_form_of_the_dir() {
        // The same name, composed and decomposed. Claude Code normalises
        // before hashing, so both are one login.
        let composed = alice("/Users/zo\u{eb}/.claude-work");
        let decomposed = alice("/Users/zoe\u{308}/.claude-work");
        assert_eq!(
            composed.keychain_services(),
            vec!["Claude Code-credentials-5e4465c4".to_string()]
        );
        assert_eq!(composed.keychain_services(), decomposed.keychain_services());
    }

    #[test]
    fn a_tilde_config_dir_is_hashed_as_the_shell_expanded_it() {
        let login = alice("~/.claude-work");
        assert_eq!(login.config_dir(), Some("/Users/alice/.claude-work"));
        assert_eq!(
            login.keychain_services()[0],
            "Claude Code-credentials-be865d75"
        );
        assert_eq!(
            login.credentials_path(),
            PathBuf::from("/Users/alice/.claude-work/.credentials.json")
        );
    }

    #[test]
    fn an_empty_config_dir_is_the_default_login() {
        let login = ClaudeLogin::with_home(Some(String::new()), Some("/Users/alice".into()));
        assert!(login.is_default());
    }

    #[test]
    fn a_default_login_reads_the_unsuffixed_item_then_the_file_in_the_home() {
        let home = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(home.path().join(".claude")).expect("mkdir");
        let file = home.path().join(".claude").join(".credentials.json");
        std::fs::write(&file, SAMPLE).expect("write");

        let login = ClaudeLogin::with_home(None, Some(home.path().to_path_buf()));
        assert_eq!(login.credentials_path(), file);

        let (lookup, asked) = keychain(&[]);
        let creds = login.read_with(Some(&lookup)).expect("reads the file");
        assert_eq!(*asked.borrow(), vec!["Claude Code-credentials".to_string()]);
        assert_eq!(creds.source, file.display().to_string());
    }

    #[test]
    fn the_default_logins_file_is_under_the_users_own_home() {
        let Some(home) = dirs::home_dir() else { return };
        assert_eq!(
            ClaudeLogin::default_login().credentials_path(),
            home.join(".claude").join(".credentials.json")
        );
        assert_eq!(
            credentials_path(),
            home.join(".claude").join(".credentials.json")
        );
    }

    #[test]
    fn an_additional_login_reads_the_credentials_file_in_its_own_dir() {
        let home = tempfile::tempdir().expect("tempdir");
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join(".credentials.json");
        std::fs::write(&file, SAMPLE).expect("write");

        let login = ClaudeLogin::with_home(
            Some(dir.path().display().to_string()),
            Some(home.path().to_path_buf()),
        );
        let creds = login.read_with(None).expect("reads its own file");
        assert_eq!(creds.source, file.display().to_string());
    }

    #[test]
    fn a_login_read_without_the_keychain_reads_its_own_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join(".credentials.json");
        std::fs::write(&file, SAMPLE).expect("write");

        let login =
            ClaudeLogin::with_home(Some(dir.path().display().to_string()), None).without_keychain();
        let creds = login.read().expect("reads the planted file");
        assert_eq!(creds.source, file.display().to_string());
        assert_eq!(
            login.keychain_services(),
            ClaudeLogin::with_home(Some(dir.path().display().to_string()), None)
                .keychain_services(),
            "it is still the same login, with the same Keychain name"
        );
    }

    #[test]
    fn the_keychain_item_wins_over_the_file_as_it_does_for_claude_code() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(".credentials.json"), "a stale leftover").expect("write");
        let login = ClaudeLogin::with_home(Some(dir.path().display().to_string()), None);
        let service = login.keychain_services()[0].clone();

        let (lookup, _) = keychain(&[(service.as_str(), SAMPLE)]);
        let creds = login.read_with(Some(&lookup)).expect("reads the Keychain");
        assert_eq!(creds.source, format!("Keychain:{service}"));
    }

    #[test]
    fn a_login_with_no_credential_names_its_dir_and_the_command_that_fixes_it() {
        let home = tempfile::tempdir().expect("tempdir");
        let dir = home.path().join(".claude-work");
        let login = ClaudeLogin::with_home(
            Some(dir.display().to_string()),
            Some(home.path().to_path_buf()),
        );
        let (lookup, _) = keychain(&[]);
        let err = login.read_with(Some(&lookup)).expect_err("nothing there");
        assert!(matches!(err, CredentialError::NotLoggedIn { .. }));
        let message = err.to_string();
        assert!(message.contains("for ~/.claude-work"), "{message}");
        assert!(
            message.contains("`CLAUDE_CONFIG_DIR=~/.claude-work claude`"),
            "{message}"
        );
        assert!(message.contains(&login.keychain_services()[0]), "{message}");
    }

    #[test]
    fn a_default_login_with_no_credential_says_to_run_claude() {
        let home = tempfile::tempdir().expect("tempdir");
        let login = ClaudeLogin::with_home(None, Some(home.path().to_path_buf()));
        let (lookup, _) = keychain(&[]);
        let message = login
            .read_with(Some(&lookup))
            .expect_err("nothing there")
            .to_string();
        assert!(
            message.contains("not logged in on this machine"),
            "{message}"
        );
        assert!(
            message.contains("Keychain:Claude Code-credentials,"),
            "{message}"
        );
        assert!(message.contains("run `claude` to log in"), "{message}");
    }

    #[test]
    fn an_unreadable_store_for_a_login_says_how_to_log_it_in_again() {
        let (lookup, _) = keychain(&[("Claude Code-credentials-be865d75", "{}")]);
        let err = alice("/Users/alice/.claude-work")
            .read_with(Some(&lookup))
            .expect_err("not a credential");
        assert!(matches!(err, CredentialError::Malformed { .. }));
        assert!(
            err.to_string()
                .contains("`CLAUDE_CONFIG_DIR=~/.claude-work claude`"),
            "{err}"
        );
    }

    #[test]
    fn a_login_command_names_its_config_dir_with_the_home_abbreviated() {
        assert_eq!(
            alice("/Users/alice/.claude-work").login_command(),
            "CLAUDE_CONFIG_DIR=~/.claude-work claude"
        );
        assert_eq!(
            alice("/srv/claude-work").login_command(),
            "CLAUDE_CONFIG_DIR=/srv/claude-work claude"
        );
        assert_eq!(
            ClaudeLogin::with_home(None, Some("/Users/alice".into())).login_command(),
            "claude"
        );
    }

    #[test]
    fn a_login_command_for_a_dir_with_a_space_survives_the_shell() {
        assert_eq!(
            alice("/Users/alice/my claude").login_command(),
            "CLAUDE_CONFIG_DIR=~/'my claude' claude"
        );
    }

    #[test]
    fn a_login_is_described_by_its_dir_and_a_default_login_by_nothing() {
        let work = alice("/Users/alice/.claude-work");
        assert_eq!(work.display_location(), "~/.claude-work");
        assert_eq!(
            work.describe(false).connect_command(),
            "ironwire connect claude --config-dir ~/.claude-work"
        );

        let default = ClaudeLogin::with_home(None, Some("/Users/alice".into()));
        assert_eq!(default.display_location(), "~/.claude");
        let described = default.describe(true);
        assert_eq!(described.product, SubscriptionProduct::Claude);
        assert_eq!(described.config_dir, None);
        assert_eq!(described.location, None);
        assert_eq!(
            described.source_dir.as_deref(),
            Some("/Users/alice/.claude")
        );
        assert!(described.default);
    }

    /// Consent is bound to the string Claude Code hashes, not to how it is
    /// shown: the abbreviated form cannot tell `~/.claude-work` from
    /// `~/.claude-work/`, which are two Keychain items.
    #[test]
    fn a_login_is_consented_to_by_the_exact_dir_claude_code_sees() {
        let moved_default = alice("~/.claude-main").describe(true);
        assert_eq!(moved_default.location.as_deref(), Some("~/.claude-main"));
        assert_eq!(
            moved_default.consent_location(),
            Some("/Users/alice/.claude-main"),
            "a relocated claude-sub is consented to by its directory"
        );

        let slashed = alice("~/.claude-work/");
        assert_eq!(
            slashed.describe(false).consent_location(),
            Some("/Users/alice/.claude-work/")
        );
        assert_eq!(slashed.consent_location(), slashed.config_dir());

        let usual = ClaudeLogin::with_home(None, Some("/Users/alice".into()));
        assert_eq!(usual.consent_location(), None);
        assert_eq!(usual.describe(true).consent_location(), None);
    }
}
