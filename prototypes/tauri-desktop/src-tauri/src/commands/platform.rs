use std::{
    ffi::OsStr,
    path::{Component, Path, PathBuf},
    process::Command,
};

use serde_json::json;
use tauri::{AppHandle, Runtime, State};

use crate::state::AppState;

fn external_url_is_allowed(url: &str) -> bool {
    let allowed_origin = near_credits_url_is_allowed(url)
        || tracecommons_run_url_is_allowed(url)
        || tracecommons_fixture_url_is_allowed(url)
        || url.starts_with("http://127.0.0.1:");
    allowed_origin
        && url.len() <= 2048
        && url.is_ascii()
        && !url
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
}

fn near_credits_url_is_allowed(url: &str) -> bool {
    let prefix = "https://cloud.near.ai/dashboard/organizations/";
    let Some(rest) = url.strip_prefix(prefix) else {
        return false;
    };
    let Some(organization_id) = rest.strip_suffix("/credits") else {
        return false;
    };
    !organization_id.is_empty()
        && organization_id.len() <= 128
        && organization_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

#[tauri::command]
pub(crate) fn open_external_url(url: String) -> Result<(), String> {
    if !external_url_is_allowed(&url) {
        return Err("external-url-not-allowed".to_owned());
    }

    open_url(&url)
}

#[tauri::command]
pub(crate) fn open_native_wallet_url(url: String, commons: String) -> Result<(), String> {
    if !native_wallet_url_is_allowed(&commons, &url) {
        return Err("native-wallet-url-not-allowed".to_owned());
    }

    open_url(&url)
}

#[tauri::command]
pub(crate) fn platform_capabilities<R: Runtime>(app: AppHandle<R>) -> serde_json::Value {
    serde_json::json!({
        "schema_version": "trace_commons.platform.v1",
        "os": std::env::consts::OS,
        "package": {
            "name": app.package_info().name,
            "version": app.package_info().version.to_string(),
            "identifier": app.config().identifier,
            "bundled": app.config().bundle.active,
        },
        "startup": crate::native::login_item_capability(),
        "notifications": crate::native::notification_capability(),
        "updates": update_state(),
        "deep_links": {
            "state": if app.config().bundle.active { "configured" } else { "unknown" },
            "scheme": "tracecommons",
        },
        "tray": { "state": "available" },
    })
}

#[tauri::command]
pub(crate) fn notification_permission() -> serde_json::Value {
    crate::native::notification_capability()
}

#[tauri::command]
pub(crate) fn request_notification_permission() -> Result<serde_json::Value, String> {
    crate::native::request_notification_permission()
}

#[tauri::command]
pub(crate) fn set_start_at_login(enabled: bool) -> Result<serde_json::Value, String> {
    crate::native::set_login_item(enabled)
}

fn update_state() -> serde_json::Value {
    if std::env::current_exe()
        .ok()
        .is_some_and(|path| installed_by_homebrew(&path))
    {
        return serde_json::json!({
            "state": "managed",
            "owner": "homebrew",
            "action": "brew upgrade --cask trace-commons",
            "feed_configured": false,
        });
    }
    serde_json::json!({
        "state": "unmanaged",
        "owner": "installer",
        "action": "install-new-version",
        "feed_configured": false,
    })
}

fn installed_by_homebrew(path: &Path) -> bool {
    path.components()
        .collect::<Vec<_>>()
        .windows(2)
        .any(|pair| {
            matches!(
                pair,
                [
                    Component::Normal(parent),
                    Component::Normal(cask)
                    ] if *parent == OsStr::new("Caskroom") && *cask == OsStr::new("trace-commons")
            )
        })
}

#[tauri::command]
pub(crate) fn update_status() -> serde_json::Value {
    update_state()
}

#[tauri::command]
pub(crate) fn check_for_update() -> serde_json::Value {
    update_state()
}

#[tauri::command]
pub(crate) fn apply_update() -> serde_json::Value {
    update_state()
}

#[tauri::command]
pub(crate) fn open_system_settings(area: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let url = match area.as_str() {
        "notifications" => "x-apple.systempreferences:com.apple.Notifications-Settings.extension",
        "login_items" => "x-apple.systempreferences:com.apple.LoginItems-Settings.extension",
        _ => return Err("system-settings-area-invalid".to_owned()),
    };
    #[cfg(not(target_os = "macos"))]
    {
        let _ = area;
        return Err("system-settings-unavailable".to_owned());
    }
    #[cfg(target_os = "macos")]
    open_url(url)
}

#[tauri::command]
pub(crate) fn quit_app<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.inner().authorize_exit();
    app.exit(0);
    Ok(())
}

fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let status = Command::new("open").arg(&url).status();
    #[cfg(target_os = "windows")]
    let status = Command::new("explorer.exe").arg(&url).status();
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let status = Command::new("xdg-open").arg(&url).status();

    status
        .map_err(|_| "external-url-open-failed".to_owned())?
        .success()
        .then_some(())
        .ok_or_else(|| "external-url-open-failed".to_owned())
}

fn https_origin(value: &str) -> Option<String> {
    let rest = value.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') || !authority.is_ascii() {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && !port.is_empty() => {
            let port = port.parse::<u16>().ok()?;
            (host, (port != 443).then_some(port))
        }
        _ => (authority, None),
    };
    if host.is_empty()
        || !host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return None;
    }
    Some(match port {
        Some(port) => format!("https://{}:{port}", host.to_ascii_lowercase()),
        None => format!("https://{}", host.to_ascii_lowercase()),
    })
}

fn native_wallet_url_is_allowed(commons: &str, url: &str) -> bool {
    url.len() <= 2048
        && url.is_ascii()
        && !url
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
        && https_origin(commons).is_some()
        && https_origin(url) == https_origin(commons)
}

pub(crate) fn is_tracecommons_deep_link(url: &str) -> bool {
    url.get(..15)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("tracecommons://"))
}

fn invite_link_is_valid(value: &str) -> bool {
    if value.len() > 2048
        || !value.is_ascii()
        || value
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return false;
    }
    let Some(rest) = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
    else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') || !authority.is_ascii() {
        return false;
    }
    let host = match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && !port.is_empty() => {
            if port.parse::<u16>().is_err() {
                return false;
            }
            host
        }
        Some(_) => return false,
        None => authority,
    };
    if host.is_empty()
        || !host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return false;
    }
    let fragment_code = value
        .split_once('#')
        .is_some_and(|(_, code)| !code.is_empty());
    let query_code = value
        .split_once('?')
        .and_then(|(_, query)| query.split('#').next())
        .into_iter()
        .flat_map(|query| query.split('&'))
        .any(|pair| {
            let Some((name, code)) = pair.split_once('=') else {
                return false;
            };
            name == "code" && !code.is_empty()
        });
    fragment_code || query_code
}

fn deep_link_invite(url: &str) -> Option<String> {
    let invite = trace_commons_contributor::commands::invite_from_deep_link(url)?;
    invite_link_is_valid(&invite).then_some(invite)
}

fn deep_link_parts(url: &str) -> Option<(&str, &str)> {
    if !is_tracecommons_deep_link(url) {
        return None;
    }
    let rest = &url[15..];
    if rest
        .chars()
        .any(|character| character.is_control() || character.is_whitespace())
    {
        return None;
    }
    let boundary = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..boundary];
    if authority.is_empty()
        || authority.contains('@')
        || !authority
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return None;
    }
    Some((authority, &rest[boundary..]))
}

fn public_run_deep_link(url: &str) -> Option<String> {
    let (authority, rest) = deep_link_parts(url)?;
    if !authority.eq_ignore_ascii_case("run") || !rest.starts_with('/') {
        return None;
    }
    let slug = &rest[1..];
    if slug.is_empty()
        || slug.len() > 63
        || !slug
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        || !slug
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        || slug.contains('/')
        || slug.contains('?')
        || slug.contains('#')
        || !slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return None;
    }
    Some(slug.to_owned())
}

fn credential_deep_link(url: &str) -> Option<String> {
    let (authority, rest) = deep_link_parts(url)?;
    if !authority.eq_ignore_ascii_case("credential") {
        return None;
    }
    if rest.contains('?') || rest.contains('#') {
        return None;
    }
    let provider = rest.strip_prefix('/').unwrap_or_default();
    if provider.contains('/')
        || (!provider.is_empty()
            && !provider
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_'))
    {
        return None;
    }
    Some(if provider.is_empty() {
        "near_ai".to_owned()
    } else {
        provider.to_owned()
    })
}

fn review_deep_link(url: &str) -> bool {
    deep_link_parts(url).is_some_and(|(authority, rest)| {
        authority.eq_ignore_ascii_case("review") && rest.is_empty()
    })
}

#[tauri::command]
pub(crate) fn consume_deep_link(
    state: State<'_, AppState>,
    url: Option<String>,
) -> Result<Option<serde_json::Value>, String> {
    let candidate = url.or_else(|| state.inner().take_pending_deep_link());
    let Some(candidate) = candidate else {
        return Ok(None);
    };
    if let Some(invite) = deep_link_invite(&candidate) {
        return Ok(Some(json!({ "kind": "enroll", "invite": invite })));
    }
    if let Some(slug) = public_run_deep_link(&candidate) {
        return Ok(Some(json!({
            "kind": "public_run",
            "slug": slug,
            "url": format!("https://tracecommons.ai/runs/{slug}"),
        })));
    }
    if let Some(provider) = credential_deep_link(&candidate) {
        return Ok(Some(json!({
            "kind": "credential",
            "provider": provider,
        })));
    }
    if review_deep_link(&candidate) {
        return Ok(Some(json!({
            "kind": "navigate",
            "path": "/waiting",
        })));
    }
    Err("deep-link-invalid".to_owned())
}

fn tracecommons_run_url_is_allowed(url: &str) -> bool {
    let prefix = "https://tracecommons.ai/runs/";
    let Some(slug) = url.strip_prefix(prefix) else {
        return false;
    };
    !slug.is_empty()
        && slug.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

fn tracecommons_fixture_url_is_allowed(url: &str) -> bool {
    let prefix = "https://github.com/TraceCommons/trace-commons/commit/";
    let Some(commit) = url.strip_prefix(prefix) else {
        return false;
    };
    commit.len() == 40 && commit.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(crate) fn existing_directory(path: &str, error: &'static str) -> Result<PathBuf, String> {
    let candidate = path.trim();
    if candidate.is_empty() {
        return Err(error.to_owned());
    }
    let path = PathBuf::from(candidate);
    if !path.is_absolute() {
        return Err(error.to_owned());
    }
    let canonical = std::fs::canonicalize(path).map_err(|_| error.to_owned())?;
    if !canonical.is_dir() {
        return Err(error.to_owned());
    }
    Ok(canonical)
}

pub(crate) fn git_repository(path: &str) -> Result<PathBuf, String> {
    let repository = existing_directory(path, "git-repository-required")?;
    let dot_git = repository.join(".git");
    if !dot_git.is_dir() && !dot_git.is_file() {
        return Err("git-repository-required".to_owned());
    }
    Ok(repository)
}

fn picker_result(output: std::process::Output) -> Result<String, String> {
    if !output.status.success() {
        return Err("directory-picker-cancelled".to_owned());
    }
    let path =
        String::from_utf8(output.stdout).map_err(|_| "directory-picker-invalid".to_owned())?;
    let path = path.trim();
    if path.is_empty() {
        return Err("directory-picker-cancelled".to_owned());
    }
    Ok(path.to_owned())
}

#[tauri::command]
pub(crate) fn pick_directory(purpose: String) -> Result<String, String> {
    let prompt = match purpose.as_str() {
        "repository" => "Choose a Git repository",
        "source_root" => "Choose a session folder",
        _ => return Err("directory-picker-purpose-invalid".to_owned()),
    };

    #[cfg(target_os = "macos")]
    let output = Command::new("osascript")
        .args([
            "-e",
            &format!("POSIX path of (choose folder with prompt {:?})", prompt),
        ])
        .output()
        .map_err(|_| "directory-picker-unavailable".to_owned())?;

    #[cfg(target_os = "windows")]
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!(
                "Add-Type -AssemblyName System.Windows.Forms; $dialog = New-Object System.Windows.Forms.FolderBrowserDialog; $dialog.Description = '{}'; if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {{ $dialog.SelectedPath }}",
                prompt.replace('\'', "''")
            ),
        ])
        .output()
        .map_err(|_| "directory-picker-unavailable".to_owned())?;

    #[cfg(target_os = "linux")]
    let output = {
        let zenity = Command::new("zenity")
            .args(["--file-selection", "--directory", "--title", prompt])
            .output();
        match zenity {
            Ok(output) => output,
            Err(_) => Command::new("kdialog")
                .args(["--getexistingdirectory", ".", prompt])
                .output()
                .map_err(|_| "directory-picker-unavailable".to_owned())?,
        }
    };

    let path = picker_result(output)?;
    Ok(existing_directory(&path, "directory-picker-invalid")?
        .to_string_lossy()
        .into_owned())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        credential_deep_link, deep_link_invite, existing_directory, external_url_is_allowed,
        git_repository, installed_by_homebrew, native_wallet_url_is_allowed,
        near_credits_url_is_allowed, public_run_deep_link, review_deep_link,
        tracecommons_fixture_url_is_allowed, tracecommons_run_url_is_allowed,
    };

    #[test]
    fn external_urls_are_limited_to_rust_owned_destinations() {
        assert!(external_url_is_allowed(
            "http://127.0.0.1:49152/near-ai/callback?state=abc"
        ));
        assert!(external_url_is_allowed(
            "https://cloud.near.ai/dashboard/organizations/example/credits"
        ));
        assert!(near_credits_url_is_allowed(
            "https://cloud.near.ai/dashboard/organizations/example/credits"
        ));
        assert!(!near_credits_url_is_allowed(
            "https://cloud.near.ai/dashboard/organizations/example/credits?next=https://example.com"
        ));
        assert!(!near_credits_url_is_allowed(
            "https://cloud.near.ai/dashboard/organizations/../other/credits"
        ));
        assert!(tracecommons_run_url_is_allowed(
            "https://tracecommons.ai/runs/repair-a-stalled-upload"
        ));
        assert!(!tracecommons_run_url_is_allowed(
            "https://tracecommons.ai/runs/repair-a-stalled-upload?next=https://example.com"
        ));
        assert!(tracecommons_fixture_url_is_allowed(
            "https://github.com/TraceCommons/trace-commons/commit/b6722426bb4b83d90425494b664ac468d67943b5"
        ));
        assert!(!tracecommons_fixture_url_is_allowed(
            "https://github.com/TraceCommons/trace-commons/commit/b6722426bb4b83d90425494b664ac468d67943b5?next=https://example.com"
        ));
        assert!(!tracecommons_fixture_url_is_allowed(
            "https://github.com/tracecommons/trace-commons/commit/b6722426bb4b83d90425494b664ac468d67943b5"
        ));
        assert!(!external_url_is_allowed("https://example.com/redirect"));
        assert!(!external_url_is_allowed(
            "https://cloud.near.ai/dashboard/organizations/example/credits\nopen"
        ));
        assert!(native_wallet_url_is_allowed(
            "https://commons.example/onboard",
            "https://commons.example/wallet/start?state=1"
        ));
        assert!(!native_wallet_url_is_allowed(
            "https://commons.example/onboard",
            "https://elsewhere.example/wallet/start"
        ));
    }

    #[test]
    fn selected_directories_are_absolute_and_git_repositories_are_checked() {
        assert!(existing_directory("/", "directory-required").is_ok());
        assert_eq!(
            existing_directory("relative", "directory-required").unwrap_err(),
            "directory-required"
        );
        let workspace = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        assert!(git_repository(workspace.to_str().unwrap()).is_ok());
        assert_eq!(git_repository("/").unwrap_err(), "git-repository-required");
    }

    #[test]
    fn deep_links_only_release_nonempty_valid_invites() {
        assert_eq!(
            deep_link_invite(
                "tracecommons://enroll?invite=https%3A%2F%2Fissuer.example%2Fonboard%23CODE"
            )
            .as_deref(),
            Some("https://issuer.example/onboard#CODE")
        );
        assert_eq!(
            deep_link_invite("tracecommons://enroll?invite=").as_deref(),
            None
        );
        assert_eq!(
            deep_link_invite(
                "TraceCommons://ENROLL/?invite=https%3A%2F%2Fissuer.example%2Fonboard%23CODE"
            )
            .as_deref(),
            Some("https://issuer.example/onboard#CODE")
        );
        assert_eq!(
            deep_link_invite(
                "tracecommons://other?invite=https%3A%2F%2Fissuer.example%2Fonboard%23CODE"
            ),
            None
        );
        assert_eq!(
            deep_link_invite("tracecommons://enroll?invite=https%3A%2F%2Fissuer.example%2Fonboard"),
            None
        );
        assert_eq!(
            public_run_deep_link("tracecommons://run/repair-a-stalled-upload").as_deref(),
            Some("repair-a-stalled-upload")
        );
        assert_eq!(
            public_run_deep_link("TraceCommons://run/repair-a-stalled-upload").as_deref(),
            Some("repair-a-stalled-upload")
        );
        assert_eq!(
            public_run_deep_link("tracecommons://run/-repair-a-stalled-upload"),
            None
        );
        assert_eq!(
            public_run_deep_link("tracecommons://run/Repair-a-stalled-upload"),
            None
        );
        assert_eq!(
            public_run_deep_link("tracecommons://run/repair-a-stalled-upload?next=evil"),
            None
        );
        assert_eq!(
            credential_deep_link("tracecommons://credential/near_ai").as_deref(),
            Some("near_ai")
        );
        assert_eq!(
            credential_deep_link("tracecommons://credential/near_ai?token=secret"),
            None
        );
        assert_eq!(
            credential_deep_link("tracecommons://credential?token=secret"),
            None
        );
        assert!(review_deep_link("tracecommons://review"));
        assert!(review_deep_link("TraceCommons://REVIEW"));
        assert!(!review_deep_link("tracecommons://review?next=/settings"));
    }

    #[test]
    fn update_ownership_follows_homebrew_cask_path_only() {
        assert!(installed_by_homebrew(Path::new(
            "/opt/homebrew/Caskroom/trace-commons/0.1.0/Trace Commons.app/Contents/MacOS/app"
        )));
        assert!(installed_by_homebrew(Path::new(
            "/usr/local/Caskroom/trace-commons/current/Trace Commons.app"
        )));
        assert!(!installed_by_homebrew(Path::new(
            "/Applications/Trace Commons.app/Contents/MacOS/app"
        )));
        assert!(!installed_by_homebrew(Path::new(
            "/opt/homebrew/Caskroom/other-app/current/app"
        )));
    }
}
