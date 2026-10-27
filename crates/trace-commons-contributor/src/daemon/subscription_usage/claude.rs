//! Manual, read-only Claude OAuth usage. Native credentials stay on this side
//! of IPC; no credential refresh, persistence, CLI launch, or cookie fallback.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::OwnedSemaphorePermit;
use trace_commons_contributor_platform::external_claude_credentials as external;

use super::providers::{failure, fingerprint, native_read};
use super::{Identity, LoginSpec, ProbeFailure, Reading, Window, file_revision};

const MAX_BYTES: usize = 1024 * 1024;
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
const NATIVE_TIMEOUT: Duration = Duration::from_secs(180);
const USAGE: &str = "https://api.anthropic.com/api/oauth/usage?cedar_ember=1";
const LEGACY_USAGE: &str = "https://api.anthropic.com/api/oauth/usage";
const PROFILE: &str = "https://api.anthropic.com/api/oauth/profile";

pub(super) fn account_path(login: &LoginSpec) -> PathBuf {
    let config = login.root.join(".config.json");
    if config.is_file() {
        config
    } else if login.config_dir.as_deref().is_some_and(|v| !v.is_empty()) {
        login.root.join(".claude.json")
    } else {
        login
            .root
            .parent()
            .unwrap_or(&login.root)
            .join(".claude.json")
    }
}

fn credential_path(login: &LoginSpec) -> PathBuf {
    let root = match login.secure_storage_dir.as_deref() {
        Some("") => dirs::home_dir()
            .map(|home| home.join(".claude"))
            .unwrap_or_else(|| login.root.clone()),
        Some(dir) => PathBuf::from(dir),
        None => login.root.clone(),
    };
    root.join(".credentials.json")
}

pub(super) fn metadata_revision(login: &LoginSpec) -> String {
    // File attributes only. In particular, status never calls Security.framework.
    format!(
        "{}|{}|{}",
        file_revision(&credential_path(login)),
        file_revision(&account_path(login)),
        file_revision(&login.root.join(".config.json"))
    )
}

fn service(login: &LoginSpec) -> Result<String, &'static str> {
    let directory = login
        .secure_storage_dir
        .as_deref()
        .or(login.config_dir.as_deref());
    match directory.filter(|dir| !dir.is_empty()) {
        None => Ok("Claude Code-credentials".into()),
        Some(dir) => Ok(format!(
            "Claude Code-credentials-{}",
            &fingerprint(external::normalized_profile_directory(dir)?.as_bytes())[..8]
        )),
    }
}

// Deserialize only the required OAuth fields. Unknown refresh tokens and MCP
// secrets are skipped, never materialized as serde_json::Value or debug output.
#[derive(Deserialize)]
struct CredentialDocument<'a> {
    #[serde(rename = "claudeAiOauth", borrow)]
    oauth: Option<OAuth<'a>>,
}
#[derive(Deserialize)]
struct OAuth<'a> {
    #[serde(rename = "accessToken")]
    access_token: &'a str,
    #[serde(rename = "expiresAt")]
    expires_at: Option<i64>,
    scopes: Option<Vec<String>>,
}
struct Credential {
    token: external::SecretBytes,
    source: Source,
    revision: String,
    local_identity: Identity,
}
enum Source {
    File,
    Keychain { service: String, revision: String },
}

fn parse_credential(
    bytes: &[u8],
    now: DateTime<Utc>,
) -> Result<external::SecretBytes, &'static str> {
    let document: CredentialDocument = serde_json::from_slice(bytes).map_err(|_| "auth_missing")?;
    let oauth = document.oauth.ok_or("auth_missing")?;
    if oauth.access_token.is_empty()
        || oauth.access_token.len() > 8192
        || oauth.access_token.chars().any(char::is_control)
    {
        return Err("auth_missing");
    }
    if oauth
        .expires_at
        .is_some_and(|expiry| DateTime::from_timestamp_millis(expiry).is_none_or(|at| at <= now))
    {
        return Err("auth_expired");
    }
    if oauth
        .scopes
        .is_some_and(|scopes| !scopes.iter().any(|scope| scope == "user:profile"))
    {
        return Err("scope_missing");
    }
    Ok(external::SecretBytes::new(
        oauth.access_token.as_bytes().to_vec(),
    ))
}

fn id(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty() && v.len() <= 1024 && !v.chars().any(char::is_control))
        .map(|v| fingerprint(v.to_ascii_lowercase().as_bytes()))
}

fn organization_id(value: Option<&Value>) -> Option<String> {
    id(value).map(|hash| fingerprint(format!("claude:organization:{hash}").as_bytes()))
}

fn profile_identity(value: &Value) -> Result<Identity, &'static str> {
    let identity = Identity {
        organization: organization_id(
            value
                .pointer("/organization/uuid")
                .or_else(|| value.get("organizationUuid"))
                .or_else(|| value.get("organization_uuid")),
        ),
        account: id(value
            .pointer("/account/uuid")
            .or_else(|| value.get("account_uuid"))
            .or_else(|| value.get("accountUuid"))),
        email: id(value
            .pointer("/account/email_address")
            .or_else(|| value.get("email_address"))
            .or_else(|| value.get("emailAddress"))),
    };
    // Usage without stable provider identity cannot establish the capture owner.
    if identity.account.is_none() {
        return Err("identity_unavailable");
    }
    Ok(identity)
}

fn read_secret_file(path: &Path) -> Result<external::SecretBytes, &'static str> {
    let file = trace_commons_contributor_platform::local_files::open_import_file(path)
        .map_err(|_| "unavailable")?;
    let mut bytes = Vec::new();
    let result = file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes);
    let bytes = external::SecretBytes::new(bytes);
    result.map_err(|_| "unavailable")?;
    if bytes.len() > MAX_BYTES {
        return Err("invalid_response");
    }
    Ok(bytes)
}

fn local_identity(login: &LoginSpec) -> Result<Identity, &'static str> {
    #[derive(Deserialize)]
    struct Config {
        #[serde(rename = "oauthAccount")]
        account: Option<Account>,
    }
    #[derive(Deserialize)]
    struct Account {
        #[serde(rename = "accountUuid")]
        uuid: Option<String>,
        #[serde(rename = "emailAddress")]
        email: Option<String>,
        #[serde(rename = "organizationUuid")]
        organization: Option<String>,
    }
    fn fingerprint_field(value: Option<String>) -> Option<String> {
        value
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty() && v.len() <= 1024 && !v.chars().any(char::is_control))
            .map(|v| fingerprint(v.to_ascii_lowercase().as_bytes()))
    }
    let path = account_path(login);
    if !path.exists() {
        return Ok(Identity::default());
    }
    let bytes = read_secret_file(&path)?;
    let config: Config = serde_json::from_slice(&bytes).map_err(|_| "identity_unavailable")?;
    let account = config.account.ok_or("auth_missing")?;
    Ok(Identity {
        account: fingerprint_field(account.uuid),
        email: fingerprint_field(account.email),
        organization: fingerprint_field(account.organization)
            .map(|hash| fingerprint(format!("claude:organization:{hash}").as_bytes())),
    })
}

fn corroborate_profile(local: &Identity, remote: &Identity) -> Result<(), &'static str> {
    if local.conflicts_with(remote) {
        return Err("auth_changed");
    }
    // Missing organization evidence is unknown, not proof that an identified
    // local organization matches the selected OAuth credential.
    if local.organization.is_some() && remote.organization.is_none() {
        return Err("identity_unavailable");
    }
    Ok(())
}

fn load(login: &LoginSpec, interactive: bool) -> Result<Credential, &'static str> {
    let revision = metadata_revision(login);
    let local_identity = local_identity(login)?;
    let path = credential_path(login);
    let (token, source) = if path.exists() {
        // A present selected-profile file is authoritative, even when expired.
        // Never substitute a different legacy credential on parse/auth failure.
        (
            parse_credential(&read_secret_file(&path)?, Utc::now())?,
            Source::File,
        )
    } else {
        if !interactive {
            return Err("keychain_needs_permission");
        }
        let service = service(login)?;
        let before = external::version(&service)?.ok_or("auth_missing")?;
        let bytes = external::read(&service, interactive)?.ok_or("auth_missing")?;
        if external::version(&service)?.as_ref() != Some(&before) {
            return Err("auth_changed");
        }
        (
            parse_credential(&bytes, Utc::now())?,
            Source::Keychain {
                service,
                revision: before,
            },
        )
    };
    if metadata_revision(login) != revision {
        return Err("auth_changed");
    }
    Ok(Credential {
        token,
        source,
        revision,
        local_identity,
    })
}

fn unchanged(login: &LoginSpec, credential: &Credential) -> Result<bool, &'static str> {
    if metadata_revision(login) != credential.revision {
        return Ok(false);
    }
    match &credential.source {
        Source::File => Ok(true),
        Source::Keychain { service, revision } => {
            Ok(external::version(service)?.as_ref() == Some(revision))
        }
    }
}

#[derive(Clone, Copy)]
enum Endpoint {
    Usage,
    LegacyUsage,
    Profile,
}
fn request(client: &reqwest::Client, token: &str, endpoint: Endpoint) -> reqwest::RequestBuilder {
    client
        .get(match endpoint {
            Endpoint::Usage => USAGE,
            Endpoint::LegacyUsage => LEGACY_USAGE,
            Endpoint::Profile => PROFILE,
        })
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
}
fn request_error(error: reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "request_timeout"
    } else {
        "unavailable"
    }
}
fn status(status: u16) -> Result<(), &'static str> {
    match status {
        200 => Ok(()),
        401 | 403 => Err("auth_expired"),
        429 => Err("rate_limited"),
        _ => Err("unavailable"),
    }
}
async fn body(mut response: reqwest::Response) -> Result<Value, &'static str> {
    status(response.status().as_u16())?;
    if response
        .content_length()
        .is_some_and(|len| len > MAX_BYTES as u64)
    {
        return Err("invalid_response");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        if chunk.len() > MAX_BYTES - bytes.len() {
            return Err("invalid_response");
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "invalid_response")
}

pub(super) async fn collect(
    login: &LoginSpec,
    interactive: bool,
    lease: Arc<OwnedSemaphorePermit>,
) -> Result<Reading, ProbeFailure> {
    let spec = login.clone();
    let credential = tokio::time::timeout(
        NATIVE_TIMEOUT,
        native_read(Arc::clone(&lease), move || load(&spec, interactive)),
    )
    .await
    .map_err(|_| failure("credential_read_timeout", None))?
    .map_err(|reason| failure(reason, None))?
    .map_err(|reason| failure(reason, None))?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(HTTP_TIMEOUT)
        .no_proxy()
        .https_only(true)
        .user_agent(concat!(
            "TraceCommons/",
            env!("CARGO_PKG_VERSION"),
            " subscription-usage"
        ))
        .build()
        .map_err(|_| failure("unavailable", None))?;
    // Requests use one token and a required stable profile identity. The local
    // config is corroboration only and never substitutes for provider evidence.
    let mut observed_identity = None;
    let result = async {
        let profile = body(
            request(
                &client,
                std::str::from_utf8(&credential.token).map_err(|_| "auth_missing")?,
                Endpoint::Profile,
            )
            .send()
            .await
            .map_err(request_error)?,
        )
        .await?;
        let identity = profile_identity(&profile)?;
        observed_identity = Some(identity.clone());
        corroborate_profile(&credential.local_identity, &identity)?;
        let mut response = request(
            &client,
            std::str::from_utf8(&credential.token).map_err(|_| "auth_missing")?,
            Endpoint::Usage,
        )
        .send()
        .await
        .map_err(request_error)?;
        if matches!(response.status().as_u16(), 400 | 403) {
            response = request(
                &client,
                std::str::from_utf8(&credential.token).map_err(|_| "auth_missing")?,
                Endpoint::LegacyUsage,
            )
            .send()
            .await
            .map_err(request_error)?;
        }
        parse_usage(&body(response).await?, identity)
    }
    .await;
    let spec = login.clone();
    let revision = credential.revision.clone();
    let same = tokio::time::timeout(
        Duration::from_secs(10),
        native_read(lease, move || unchanged(&spec, &credential)),
    )
    .await
    .map_err(|_| failure("credential_check_timeout", None))?
    .map_err(|reason| failure(reason, None))?
    .map_err(|reason| failure(reason, None))?;
    if !same {
        return Err(failure("auth_changed", None));
    }
    let mut reading = result.map_err(|reason| failure(reason, observed_identity))?;
    reading.native_revision = Some(revision);
    Ok(reading)
}

fn percent(value: Option<&Value>) -> Option<u16> {
    value
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite() && (0.0..=100.0).contains(n))
        .map(|n| (n * 10.0).round() as u16)
}
fn reset(value: Option<&Value>) -> Option<DateTime<Utc>> {
    value
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|date| date.with_timezone(&Utc))
}
fn parse_usage(value: &Value, identity: Identity) -> Result<Reading, &'static str> {
    if !value.is_object() {
        return Err("invalid_response");
    }
    let mut windows = Vec::new();
    for (key, lane, label, duration) in [
        ("five_hour", "session", None, 300),
        ("seven_day", "weekly", None, 10080),
        ("seven_day_opus", "model", Some("Opus"), 10080),
        ("seven_day_sonnet", "model", Some("Sonnet"), 10080),
    ] {
        if let Some(window) = value.get(key).filter(|v| v.is_object()) {
            windows.push(Window {
                lane: lane.into(),
                label: label.map(str::to_owned),
                duration_mins: Some(duration),
                used_permille: percent(window.get("utilization")),
                resets_at: reset(window.get("resets_at")),
            });
        }
    }
    if let Some(limits) = value.get("limits").and_then(Value::as_array) {
        for entry in limits.iter().take(64) {
            if entry.get("kind").and_then(Value::as_str) != Some("weekly_scoped")
                || entry.get("group").and_then(Value::as_str) != Some("weekly")
            {
                continue;
            }
            let Some(label) = entry
                .pointer("/scope/model/display_name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty() && s.len() <= 64 && !s.chars().any(char::is_control))
            else {
                continue;
            };
            let model = entry
                .pointer("/scope/model/id")
                .and_then(Value::as_str)
                .unwrap_or(label);
            if label.eq_ignore_ascii_case("all models")
                || model.ends_with("all-models")
                || windows
                    .iter()
                    .any(|w| w.lane == "model" && w.label.as_deref() == Some(label))
            {
                continue;
            }
            windows.push(Window {
                lane: "model".into(),
                label: Some(label.into()),
                duration_mins: Some(10080),
                used_permille: percent(entry.get("percent")),
                resets_at: reset(entry.get("resets_at")),
            });
        }
    }
    windows.truncate(16);
    Ok(Reading {
        windows,
        banked_resets: None,
        identity,
        native_revision: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn login(root: &std::path::Path) -> LoginSpec {
        LoginSpec {
            provider: super::super::Provider::Claude,
            root: root.into(),
            id: "claude-fixture".into(),
            label: "Claude".into(),
            default_login: false,
            native_executable: None,
            config_dir: Some(root.to_string_lossy().into()),
            secure_storage_dir: None,
            metadata_revision: None,
            native_revision: None,
        }
    }
    #[test]
    fn credential_expiry_scope_and_mcp_only_are_not_false_sign_outs() {
        assert_eq!(parse_credential(br#"{"claudeAiOauth":{"accessToken":"fixture","expiresAt":1,"refreshToken":"ignored"}}"#, Utc::now()).unwrap_err(), "auth_expired");
        assert_eq!(
            parse_credential(
                br#"{"claudeAiOauth":{"accessToken":"fixture","scopes":["user:inference"]}}"#,
                Utc::now()
            )
            .unwrap_err(),
            "scope_missing"
        );
        assert_eq!(
            parse_credential(br#"{"mcpOAuth":{"secret":"ignored"}}"#, Utc::now()).unwrap_err(),
            "auth_missing"
        );
        assert_eq!(
            parse_credential(
                br#"{"claudeAiOauth":{"accessToken":"fixture","scopes":["user:profile"]}}"#,
                Utc::now()
            )
            .unwrap()
            .as_ref(),
            b"fixture"
        );
    }
    #[test]
    fn stable_profile_identity_survives_token_rotation_and_detects_account_switch() {
        let first = profile_identity(
            &json!({"account":{"uuid":"account-a","email_address":"a@example.invalid"}}),
        )
        .unwrap();
        let same = profile_identity(
            &json!({"accountUuid":"account-a", "emailAddress":"A@example.invalid"}),
        )
        .unwrap();
        let changed = profile_identity(&json!({"account_uuid":"account-b"})).unwrap();
        assert_eq!(first, same);
        assert!(first.conflicts_with(&changed));
        assert!(profile_identity(&json!({"emailAddress":"a@example.invalid"})).is_err());
    }
    #[test]
    fn same_account_different_organization_is_a_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let spec = login(dir.path());
        std::fs::write(
            account_path(&spec),
            br#"{"oauthAccount":{"accountUuid":"account-a","organizationUuid":"org-a"}}"#,
        )
        .unwrap();
        let local = local_identity(&spec).unwrap();
        let wrong = profile_identity(
            &json!({"account":{"uuid":"account-a"},"organization":{"uuid":"org-b"}}),
        )
        .unwrap();
        assert!(local.conflicts_with(&wrong));
        let missing = profile_identity(&json!({"account":{"uuid":"account-a"}})).unwrap();
        assert!(!local.conflicts_with(&missing));
        assert_eq!(
            corroborate_profile(&local, &missing),
            Err("identity_unavailable")
        );
        assert_eq!(corroborate_profile(&local, &wrong), Err("auth_changed"));
        let matching =
            profile_identity(&json!({"accountUuid":"account-a","organizationUuid":"org-a"}))
                .unwrap();
        assert_eq!(local, matching);
        assert!(corroborate_profile(&local, &matching).is_ok());
    }

    #[test]
    fn fixed_https_get_endpoints_never_put_tokens_in_urls() {
        let client = reqwest::Client::new();
        for endpoint in [Endpoint::Usage, Endpoint::LegacyUsage, Endpoint::Profile] {
            let req = request(&client, "fixture-secret", endpoint)
                .build()
                .unwrap();
            assert_eq!(req.method(), reqwest::Method::GET);
            assert_eq!(req.url().scheme(), "https");
            assert_eq!(req.url().host_str(), Some("api.anthropic.com"));
            assert!(!req.url().as_str().contains("fixture-secret"));
            assert_eq!(req.headers()["anthropic-beta"], "oauth-2025-04-20");
            assert!(req.body().is_none());
        }
        assert_eq!(status(401), Err("auth_expired"));
        assert_eq!(status(429), Err("rate_limited"));
        assert_eq!(status(302), Err("unavailable"));
    }
    #[tokio::test]
    async fn response_bodies_are_bounded_and_errors_never_echo_provider_content() {
        fn response(status: u16, bytes: Vec<u8>) -> reqwest::Response {
            axum::http::Response::builder()
                .status(status)
                .body(bytes)
                .unwrap()
                .into()
        }
        assert_eq!(
            body(response(
                200,
                br#"{"five_hour":{"utilization":15}}"#.to_vec()
            ))
            .await
            .unwrap()["five_hour"]["utilization"],
            15
        );
        assert_eq!(
            body(response(200, vec![b' '; MAX_BYTES + 1]))
                .await
                .unwrap_err(),
            "invalid_response"
        );
        assert_eq!(
            body(response(401, b"PRIVATE_PROVIDER_ERROR_TOKEN".to_vec()))
                .await
                .unwrap_err(),
            "auth_expired"
        );
        assert_eq!(
            body(response(200, b"PRIVATE_PROVIDER_NON_JSON".to_vec()))
                .await
                .unwrap_err(),
            "invalid_response"
        );
        assert_eq!(
            body(response(302, b"https://untrusted.invalid/SECRET".to_vec()))
                .await
                .unwrap_err(),
            "unavailable"
        );
    }

    #[test]
    fn usage_parses_reported_fable_and_preserves_unknown_values() {
        let reading = parse_usage(&json!({"five_hour":{"utilization":12.5},"seven_day":{"utilization":false}, "seven_day_sonnet":null,
            "limits":[{"kind":"weekly_scoped","group":"weekly","percent":40,"scope":{"model":{"id":"claude/fable.5:promo","display_name":"Fable"}}}]}), Identity::default()).unwrap();
        assert_eq!(reading.windows.len(), 3);
        assert_eq!(reading.windows[0].used_permille, Some(125));
        assert_eq!(reading.windows[1].used_permille, None);
        assert_eq!(reading.windows[2].label.as_deref(), Some("Fable"));
        assert_eq!(reading.windows[2].used_permille, Some(400));
        assert!(
            parse_usage(&json!({}), Identity::default())
                .unwrap()
                .windows
                .is_empty()
        );
        assert!(parse_usage(&json!([]), Identity::default()).is_err());
    }
    #[test]
    fn file_profile_changes_and_signout_invalidate_capture_witness() {
        let dir = tempfile::tempdir().unwrap();
        let spec = login(dir.path());
        std::fs::write(
            account_path(&spec),
            br#"{"oauthAccount":{"accountUuid":"a"}}"#,
        )
        .unwrap();
        std::fs::write(
            credential_path(&spec),
            br#"{"claudeAiOauth":{"accessToken":"one"}}"#,
        )
        .unwrap();
        let original = load(&spec, false).ok().unwrap();
        assert!(unchanged(&spec, &original).unwrap());
        std::fs::write(
            credential_path(&spec),
            br#"{"claudeAiOauth":{"accessToken":"two"}}"#,
        )
        .unwrap();
        assert!(!unchanged(&spec, &original).unwrap());
        let rotated = load(&spec, false).ok().unwrap();
        assert_eq!(original.local_identity, rotated.local_identity);
        std::fs::write(
            account_path(&spec),
            br#"{"oauthAccount":{"accountUuid":"b"}}"#,
        )
        .unwrap();
        assert!(!unchanged(&spec, &rotated).unwrap());
        std::fs::write(account_path(&spec), br#"{}"#).unwrap();
        assert!(matches!(load(&spec, false), Err("auth_missing")));
    }
    #[test]
    fn preferred_expired_file_never_falls_back_to_keychain() {
        let dir = tempfile::tempdir().unwrap();
        let spec = login(dir.path());
        std::fs::write(
            credential_path(&spec),
            br#"{"claudeAiOauth":{"accessToken":"one","expiresAt":1}}"#,
        )
        .unwrap();
        assert!(matches!(load(&spec, true), Err("auth_expired")));
    }
    #[test]
    fn secure_override_selects_file_and_service_without_changing_account_profile() {
        let dir = tempfile::tempdir().unwrap();
        let mut spec = login(dir.path());
        let original = account_path(&spec);
        spec.secure_storage_dir = Some("/fixture/secure".into());
        assert_eq!(
            credential_path(&spec),
            PathBuf::from("/fixture/secure/.credentials.json")
        );
        assert_eq!(account_path(&spec), original);
        assert_eq!(
            service(&spec).unwrap(),
            format!(
                "Claude Code-credentials-{}",
                &fingerprint(b"/fixture/secure")[..8]
            )
        );
        spec.secure_storage_dir = Some(String::new());
        assert_eq!(service(&spec).unwrap(), "Claude Code-credentials");
    }
}
