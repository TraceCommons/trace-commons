use std::{
    ffi::OsStr,
    path::{Component, Path, PathBuf},
    process::Command,
};

use tauri::{AppHandle, Runtime, State};

use trace_commons_contributor::quit_copy::{self, QuitRole};

use crate::state::AppState;

/// The fixed allowlist lives in the core (K7, #1173:
/// `trace_commons_contributor::external_url::is_allowed`), unchanged, so
/// every shell opens only the same allowed hosts.
fn external_url_is_allowed(url: &str) -> bool {
    trace_commons_contributor::external_url::is_allowed(url)
}

#[tauri::command]
pub(crate) fn open_external_url(url: String) -> Result<(), String> {
    if !external_url_is_allowed(&url) {
        return Err("external-url-not-allowed".to_owned());
    }

    open_url(&url)
}

#[tauri::command]
pub(crate) fn open_native_wallet_url(
    url: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if !wallet_url_is_valid(&url) || !state.inner().consume_wallet_url(&url)? {
        return Err("native-wallet-url-not-allowed".to_owned());
    }

    open_url(&url)
}

#[tauri::command]
pub(crate) fn open_account_sign_in_url(
    url: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if !wallet_url_is_valid(&url) || !state.inner().consume_account_sign_in_url(&url)? {
        return Err("account-sign-in-url-not-allowed".to_owned());
    }

    open_url(&url)
}

#[tauri::command]
pub(crate) async fn platform_capabilities<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let (startup, notifications) = tauri::async_runtime::spawn_blocking(|| {
        (
            crate::native::login_item_capability(),
            crate::native::notification_capability(),
        )
    })
    .await
    .map_err(|_| "platform-capabilities-unavailable".to_owned())?;
    Ok(serde_json::json!({
        "schema_version": "trace_commons.platform.v1",
        "os": std::env::consts::OS,
        "package": {
            "name": app.package_info().name,
            "version": app.package_info().version.to_string(),
            "identifier": app.config().identifier,
            "bundled": app.config().bundle.active,
        },
        "startup": startup,
        "notifications": notifications,
        "updates": update_state(),
        "deep_links": {
            "state": state.inner().deep_link_state(),
            "scheme": "tracecommons",
        },
        "tray": { "state": "available" },
    }))
}

#[tauri::command]
pub(crate) async fn notification_permission() -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(crate::native::notification_capability)
        .await
        .map_err(|_| "notification-permission-unavailable".to_owned())
}

#[tauri::command]
pub(crate) async fn request_notification_permission() -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(crate::native::request_notification_permission)
        .await
        .map_err(|_| "notification-permission-unavailable".to_owned())?
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
        Err("system-settings-unavailable".to_owned())
    }
    #[cfg(target_os = "macos")]
    open_url(url)
}

/// The quit prompt that is true for this process: hosting the watcher,
/// attached to one another process runs, or connected to none.
#[tauri::command]
pub(crate) fn quit_confirmation_copy(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    quit_prompt_value(state.inner().quit_role())
}

fn quit_prompt_value(role: QuitRole) -> Result<serde_json::Value, String> {
    serde_json::to_value(quit_copy::quit_prompt(role))
        .map_err(|_| "quit-copy-unavailable".to_owned())
}

/// The status lines a screen shows around a failed read or request, and
/// the core-down banner, as the core words them. Needs no daemon: these are
/// what a screen shows when the daemon did not answer.
#[tauri::command]
pub(crate) fn shell_status_copy() -> Result<serde_json::Value, String> {
    serde_json::to_value(trace_commons_contributor::health_copy::shell_status_copy())
        .map_err(|_| "shell-status-copy-unavailable".to_owned())
}

#[tauri::command]
pub(crate) fn quit_app<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    app.exit(0);
    Ok(())
}

fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let status = Command::new("open").arg(url).status();
    #[cfg(target_os = "windows")]
    return Command::new("explorer.exe")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|_| "external-url-open-failed".to_owned());
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let status = Command::new("xdg-open").arg(url).status();

    #[cfg(not(target_os = "windows"))]
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

fn wallet_url_is_valid(url: &str) -> bool {
    url.len() <= 2048
        && url.is_ascii()
        && !url
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
        && https_origin(url).is_some()
}

// Parsing lives in the contributor core (K7, #1173): `is_tracecommons_deep_link`
// and `REVIEW_DEEP_LINK` are used elsewhere in this crate as `platform::`, so
// they are re-exported here rather than changed at each call site.
pub(crate) use trace_commons_contributor::deep_link::{
    REVIEW_DEEP_LINK, is_tracecommons_deep_link,
};

/// Parse a pending or just-delivered deep link into the action the frontend
/// should take. Parsing itself -- scheme, host, path, and which action each
/// shape names -- is `deep_link::parse_deep_link`, a pure function the core
/// also exports through the C ABI so macOS parses identically; this command
/// only supplies the pending link Tauri itself stored (see
/// `AppState::take_pending_deep_link` and `app::remember_deep_link`) and
/// turns the parser's typed action into the JSON the frontend has always
/// read.
#[tauri::command]
pub(crate) fn consume_deep_link(
    state: State<'_, AppState>,
    url: Option<String>,
) -> Result<Option<serde_json::Value>, String> {
    let candidate = url.or_else(|| state.inner().take_pending_deep_link());
    let Some(candidate) = candidate else {
        return Ok(None);
    };
    trace_commons_contributor::deep_link::parse_deep_link(&candidate)
        .map(|action| Some(serde_json::to_value(action).expect("DeepLinkAction always serializes")))
        .map_err(str::to_owned)
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
pub(crate) async fn pick_directory(purpose: String) -> Result<String, String> {
    let prompt = match purpose.as_str() {
        "repository" => "Choose a Git repository",
        "source_root" => "Choose a trace folder",
        _ => return Err("directory-picker-purpose-invalid".to_owned()),
    };

    tauri::async_runtime::spawn_blocking(move || pick_directory_blocking(prompt))
        .await
        .map_err(|_| "directory-picker-unavailable".to_owned())?
}

fn pick_directory_blocking(prompt: &'static str) -> Result<String, String> {
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
        REVIEW_DEEP_LINK, existing_directory, external_url_is_allowed, git_repository,
        installed_by_homebrew, is_tracecommons_deep_link, quit_prompt_value, wallet_url_is_valid,
    };
    use trace_commons_contributor::quit_copy::{self, QuitRole};

    #[test]
    fn quit_prompt_reaches_the_frontend_with_the_role_it_was_chosen_for() {
        for (role, label, body) in [
            (QuitRole::Hosting, "hosting", quit_copy::QUIT_HOSTING_BODY),
            (
                QuitRole::Attached,
                "attached",
                quit_copy::QUIT_ATTACHED_BODY,
            ),
            (
                QuitRole::Unavailable,
                "unavailable",
                quit_copy::QUIT_UNAVAILABLE_BODY,
            ),
        ] {
            let value = quit_prompt_value(role).unwrap();
            assert_eq!(value["role"], label);
            assert_eq!(value["body"], body);
            assert_eq!(value["title"], quit_copy::QUIT_TITLE);
            assert_eq!(value["confirm"], quit_copy::QUIT_CONFIRM);
            assert_eq!(value["cancel"], quit_copy::QUIT_CANCEL);
        }
    }

    #[test]
    fn shell_status_lines_reach_the_frontend_as_the_core_words_them() {
        use trace_commons_contributor::{health_copy, preview_copy};
        let value = super::shell_status_copy().unwrap();
        let core_down = health_copy::core_down_copy();
        assert_eq!(value["core_down"]["title"], core_down.title);
        assert_eq!(value["core_down"]["detail"], core_down.detail);
        assert_eq!(value["read_unavailable"], health_copy::READ_UNAVAILABLE);
        assert_eq!(
            value["request_failed"],
            preview_copy::MONITOR_REQUEST_FAILED
        );
        assert_eq!(value["retry_startup"], health_copy::RETRY_STARTUP);
        assert_eq!(value["retrying_startup"], health_copy::RETRYING_STARTUP);
    }

    // The allowlist itself moved to the contributor core (K7, #1173:
    // `trace_commons_contributor::external_url`), which has its own
    // exhaustive test module, including the negative cases (lookalike
    // hosts, `javascript:`/`data:` schemes, over-long and garbage input,
    // embedded control characters). This is only the wiring check:
    // `external_url_is_allowed` is a thin call-through, unchanged for a
    // representative allowed and refused URL.
    #[test]
    fn external_url_allowlist_is_the_cores_reached_through_the_wrapper() {
        assert!(external_url_is_allowed(
            "http://127.0.0.1:49152/near-ai/callback?state=abc"
        ));
        assert!(external_url_is_allowed(
            "https://cloud.near.ai/dashboard/organizations/example/credits"
        ));
        assert!(!external_url_is_allowed("https://example.com/redirect"));
        assert!(!external_url_is_allowed(
            "http://127.0.0.1:49152@attacker.example/near-ai/callback"
        ));
    }

    // Unrelated to the allowlist above: the wallet/account-sign-in URL
    // checker validates against a nonce the shell itself minted, not a
    // fixed host list, and was not moved.
    #[test]
    fn wallet_urls_check_origin_shape_only_here() {
        assert!(wallet_url_is_valid(
            "https://commons.example/wallet/start?state=1"
        ));
        assert!(!wallet_url_is_valid(
            "https://commons.example@attacker.example/wallet"
        ));
    }

    #[test]
    fn selected_directories_are_absolute_and_git_repositories_are_checked() {
        let temporary_directory = std::env::temp_dir();
        let temporary_directory = temporary_directory.to_str().unwrap();
        assert!(existing_directory(temporary_directory, "directory-required").is_ok());
        assert_eq!(
            existing_directory("relative", "directory-required").unwrap_err(),
            "directory-required"
        );
        let workspace = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        assert!(git_repository(workspace.to_str().unwrap()).is_ok());
        assert_eq!(
            git_repository(temporary_directory).unwrap_err(),
            "git-repository-required"
        );
    }

    // Deep-link parsing itself moved to the contributor core (K7, #1173:
    // `trace_commons_contributor::deep_link`), which has its own exhaustive
    // test module, including the negative cases (lookalike authorities,
    // non-`tracecommons` schemes, control characters, over-long garbage).
    // This is only the wiring check: `platform::is_tracecommons_deep_link`
    // and `platform::REVIEW_DEEP_LINK` are re-exports, and `consume_deep_link`
    // hands its candidate straight to `deep_link::parse_deep_link` and
    // reports the typed action or its refusal label unchanged.
    #[test]
    fn deep_link_parsing_is_the_cores_reached_through_the_re_exports() {
        use trace_commons_contributor::deep_link::{DeepLinkAction, parse_deep_link};

        assert!(is_tracecommons_deep_link(REVIEW_DEEP_LINK));
        assert!(is_tracecommons_deep_link("TraceCommons://REVIEW"));
        assert!(!is_tracecommons_deep_link("https://example.com/"));

        assert_eq!(
            parse_deep_link(REVIEW_DEEP_LINK),
            Ok(DeepLinkAction::Navigate { path: "/waiting" })
        );
        assert_eq!(
            parse_deep_link(
                "tracecommons://enroll?invite=https%3A%2F%2Fissuer.example%2Fonboard%23CODE"
            ),
            Ok(DeepLinkAction::Enroll {
                invite: "https://issuer.example/onboard#CODE".to_owned()
            })
        );
        assert!(parse_deep_link("not-a-deep-link").is_err());
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
