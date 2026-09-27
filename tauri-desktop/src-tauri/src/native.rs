use std::ffi::CString;
#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};

#[cfg(target_os = "windows")]
use std::ffi::c_void;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn tc_macos_notification_configure(on_review: extern "C" fn()) -> i32;
    fn tc_macos_notification_status() -> i32;
    fn tc_macos_request_notification_permission() -> i32;
    fn tc_macos_post_digest(body: *const std::ffi::c_char) -> i32;
    fn tc_macos_login_item_status() -> i32;
    fn tc_macos_set_login_item(enabled: i32) -> i32;
}

#[cfg(target_os = "windows")]
type HKey = *mut c_void;

#[cfg(target_os = "windows")]
#[link(name = "advapi32")]
unsafe extern "system" {
    fn RegOpenCurrentUser(sam_desired: u32, result: *mut HKey) -> i32;
    fn RegCloseKey(key: HKey) -> i32;
    fn RegGetValueW(
        key: HKey,
        sub_key: *const u16,
        value_name: *const u16,
        flags: u32,
        value_type: *mut u32,
        data: *mut c_void,
        data_size: *mut u32,
    ) -> i32;
    fn RegSetKeyValueW(
        key: HKey,
        sub_key: *const u16,
        value_name: *const u16,
        value_type: u32,
        data: *const c_void,
        data_size: u32,
    ) -> i32;
    fn RegDeleteKeyValueW(key: HKey, sub_key: *const u16, value_name: *const u16) -> i32;
}

#[cfg(target_os = "windows")]
const WINDOWS_RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(target_os = "windows")]
const WINDOWS_RUN_VALUE: &str = "TraceCommonsDesktop";
#[cfg(target_os = "windows")]
const WINDOWS_REG_SZ: u32 = 1;
#[cfg(target_os = "windows")]
const WINDOWS_RRF_RT_REG_SZ: u32 = 0x0000_0002;
#[cfg(target_os = "windows")]
const WINDOWS_KEY_QUERY_VALUE: u32 = 0x0001;
#[cfg(target_os = "windows")]
const WINDOWS_KEY_SET_VALUE: u32 = 0x0002;
#[cfg(target_os = "windows")]
const WINDOWS_ERROR_FILE_NOT_FOUND: i32 = 2;

#[cfg(target_os = "linux")]
const LINUX_SYSTEMD_UNIT_FILE_NAME: &str = "trace-commons-contributor.service";
#[cfg(target_os = "linux")]
const LINUX_XDG_AUTOSTART_FILE_NAME: &str = "ai.tracecommons.tauri.prototype.desktop";

#[cfg(target_os = "windows")]
struct CurrentUserKey(HKey);

#[cfg(target_os = "windows")]
impl CurrentUserKey {
    fn open(access: u32) -> Result<Self, i32> {
        let mut key = std::ptr::null_mut();
        // SAFETY: `key` is a valid output pointer and `access` requests only
        // the operations this helper performs against the current user's hive.
        let status = unsafe { RegOpenCurrentUser(access, &mut key) };
        if status == 0 && !key.is_null() {
            Ok(Self(key))
        } else {
            Err(status)
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for CurrentUserKey {
    fn drop(&mut self) {
        // SAFETY: this handle was returned by RegOpenCurrentUser and is closed
        // exactly once here.
        unsafe { RegCloseKey(self.0) };
    }
}

/// What a digest notification's Review click does, registered once at
/// launch. Held behind a lock rather than a `OnceLock` so a test (or a
/// second configure) replaces it instead of failing.
#[cfg(any(target_os = "macos", test))]
type ReviewRoute = std::sync::Arc<dyn Fn() + Send + Sync>;

#[cfg(any(target_os = "macos", test))]
static REVIEW_ROUTE: std::sync::Mutex<Option<ReviewRoute>> = std::sync::Mutex::new(None);

#[cfg(any(target_os = "macos", test))]
fn set_review_route(route: impl Fn() + Send + Sync + 'static) {
    let mut slot = REVIEW_ROUTE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *slot = Some(std::sync::Arc::new(route));
}

/// Called by the notification delegate in `native_macos.m` when a digest
/// notification (or its Review action) is clicked.
///
/// The click is handled inside this process instead of asking
/// LaunchServices to open `tracecommons://review`: another installed app
/// that also claims the scheme (the native macOS shell) could otherwise
/// receive the click this app's notification produced.
///
/// It must not unwind into Objective-C, so a panicking route is caught.
#[cfg(any(target_os = "macos", test))]
extern "C" fn tc_notification_review_requested() {
    let route = REVIEW_ROUTE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    if let Some(route) = route {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| route()));
    }
}

/// Register the digest category and route its Review click to `on_review`.
pub(crate) fn configure_notifications(on_review: impl Fn() + Send + Sync + 'static) {
    #[cfg(target_os = "macos")]
    {
        set_review_route(on_review);
        // The native bridge returns a fixed status only. There is no
        // user-facing failure at launch: an unsigned/dev binary may not have
        // a notification center, while the packaged app will configure its
        // category here.
        unsafe {
            let _ = tc_macos_notification_configure(tc_notification_review_requested);
        }
    }
    #[cfg(not(target_os = "macos"))]
    drop(on_review);
}

pub(crate) fn notification_capability() -> serde_json::Value {
    #[cfg(target_os = "macos")]
    let status = unsafe { tc_macos_notification_status() };
    #[cfg(not(target_os = "macos"))]
    let status = -1;
    serde_json::json!({
        "state": notification_state(status),
        "permission": status,
    })
}

pub(crate) fn request_notification_permission() -> Result<serde_json::Value, String> {
    #[cfg(target_os = "macos")]
    {
        let result = unsafe { tc_macos_request_notification_permission() };
        if result < 0 {
            return Err("notification-permission-unavailable".to_owned());
        }
        Ok(notification_capability())
    }
    #[cfg(not(target_os = "macos"))]
    Err("notification-permission-unavailable".to_owned())
}

pub(crate) fn login_item_capability() -> serde_json::Value {
    #[cfg(target_os = "macos")]
    let status = unsafe { tc_macos_login_item_status() };
    #[cfg(target_os = "windows")]
    let status = windows_login_item_status();
    #[cfg(target_os = "linux")]
    let status = linux_login_item_status();
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    let status = -1;
    serde_json::json!({
        "state": login_item_state(status),
        "status": status,
    })
}

pub(crate) fn set_login_item(enabled: bool) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "macos")]
    {
        let status = unsafe { tc_macos_set_login_item(i32::from(enabled)) };
        if status < 0 {
            return Err("start-at-login-change-failed".to_owned());
        }
        if !login_item_change_succeeded(status, enabled) {
            return Err("start-at-login-change-failed".to_owned());
        }
        Ok(login_item_capability())
    }
    #[cfg(target_os = "windows")]
    {
        let status = windows_set_login_item(enabled);
        if status < 0 || !login_item_change_succeeded(status, enabled) {
            return Err("start-at-login-change-failed".to_owned());
        }
        Ok(login_item_capability())
    }
    #[cfg(target_os = "linux")]
    {
        linux_set_login_item(enabled)?;
        Ok(login_item_capability())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = enabled;
        Err("start-at-login-unavailable".to_owned())
    }
}

pub(crate) fn post_digest(body: &str) -> Result<(), String> {
    let body = body
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .take(4000)
        .collect::<String>();
    let body = CString::new(body).map_err(|_| "notification-body-invalid".to_owned())?;

    #[cfg(target_os = "macos")]
    {
        let status = unsafe { tc_macos_notification_status() };
        if !notification_can_post(status) {
            return Ok(());
        }
        let result = unsafe { tc_macos_post_digest(body.as_ptr()) };
        (result == 0)
            .then_some(())
            .ok_or_else(|| "notification-post-failed".to_owned())
    }
    #[cfg(target_os = "linux")]
    {
        let body = body.to_string_lossy();
        let status = std::process::Command::new("notify-send")
            .args(["--app-name=Trace Commons", "--", "Trace Commons"])
            .arg(body.as_ref())
            .status()
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    "notification-post-unavailable".to_owned()
                } else {
                    "notification-post-failed".to_owned()
                }
            })?;
        status
            .success()
            .then_some(())
            .ok_or_else(|| "notification-post-failed".to_owned())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = body;
        Err("notification-post-unavailable".to_owned())
    }
}

fn notification_state(status: i32) -> &'static str {
    match status {
        0 => "requires_approval",
        1 => "denied",
        2 | 3 => "available",
        _ => "unknown",
    }
}

#[cfg(any(target_os = "macos", test))]
fn notification_can_post(status: i32) -> bool {
    matches!(status, 2 | 3)
}

fn login_item_state(status: i32) -> &'static str {
    match status {
        0 => "not_registered",
        1 => "enabled",
        2 => "requires_approval",
        3 => "not_found",
        4 => "managed",
        _ => "unknown",
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", test))]
fn login_item_change_succeeded(status: i32, enabled: bool) -> bool {
    if enabled {
        matches!(status, 1 | 2)
    } else {
        matches!(status, 0 | 3)
    }
}

#[cfg(target_os = "linux")]
fn linux_login_item_status() -> i32 {
    let Some(config_dir) = linux_config_dir() else {
        return -1;
    };
    let Ok(executable) = std::env::current_exe() else {
        return -1;
    };
    linux_login_item_status_in(&config_dir, &executable)
}

#[cfg(target_os = "linux")]
fn linux_login_item_status_in(config_dir: &Path, executable: &Path) -> i32 {
    if linux_systemd_unit_path(config_dir).exists() {
        return 4;
    }
    let Ok(expected) = linux_desktop_entry_text(executable) else {
        return -1;
    };
    match linux_entry_matches(&linux_xdg_entry_path(config_dir), &expected) {
        Ok(None) => 0,
        Ok(Some(true)) => 1,
        Ok(Some(false)) | Err(_) => -1,
    }
}

#[cfg(target_os = "linux")]
fn linux_set_login_item(enabled: bool) -> Result<(), String> {
    let config_dir = linux_config_dir().ok_or_else(|| "start-at-login-unavailable".to_owned())?;
    let executable =
        std::env::current_exe().map_err(|_| "start-at-login-unavailable".to_owned())?;
    linux_set_login_item_in(&config_dir, &executable, enabled)
}

#[cfg(target_os = "linux")]
fn linux_set_login_item_in(
    config_dir: &Path,
    executable: &Path,
    enabled: bool,
) -> Result<(), String> {
    if linux_systemd_unit_path(config_dir).exists() {
        return if enabled {
            // The daemon's installed systemd unit already starts it at login.
            // Never add an XDG entry alongside that independent launcher.
            Ok(())
        } else {
            Err("start-at-login-managed-by-systemd".to_owned())
        };
    }

    let path = linux_xdg_entry_path(config_dir);
    let expected = linux_desktop_entry_text(executable)?;
    if enabled {
        if let Some(matches) = linux_entry_matches(&path, &expected)
            .map_err(|_| "start-at-login-change-failed".to_owned())?
        {
            return if matches {
                Ok(())
            } else {
                Err("start-at-login-entry-conflict".to_owned())
            };
        }

        let parent = path
            .parent()
            .ok_or_else(|| "start-at-login-change-failed".to_owned())?;
        std::fs::create_dir_all(parent).map_err(|_| "start-at-login-change-failed".to_owned())?;
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true).mode(0o644);
        match options.open(&path) {
            Ok(mut file) => file
                .write_all(expected.as_bytes())
                .map_err(|_| "start-at-login-change-failed".to_owned()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                match linux_entry_matches(&path, &expected) {
                    Ok(Some(true)) => Ok(()),
                    _ => Err("start-at-login-entry-conflict".to_owned()),
                }
            }
            Err(_) => Err("start-at-login-change-failed".to_owned()),
        }
    } else {
        match linux_entry_matches(&path, &expected)
            .map_err(|_| "start-at-login-change-failed".to_owned())?
        {
            None => Ok(()),
            Some(true) => {
                std::fs::remove_file(&path).map_err(|_| "start-at-login-change-failed".to_owned())
            }
            Some(false) => Err("start-at-login-entry-conflict".to_owned()),
        }
    }
}

/// `Ok(None)` means absent; `Ok(Some(false))` means a file exists but is not
/// the exact entry this adapter owns. Symlinks and directories are conflicts.
#[cfg(target_os = "linux")]
fn linux_entry_matches(path: &Path, expected: &str) -> std::io::Result<Option<bool>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_file() {
        return Ok(Some(false));
    }
    Ok(Some(std::fs::read_to_string(path)? == expected))
}

#[cfg(target_os = "linux")]
fn linux_config_dir() -> Option<PathBuf> {
    if let Some(config_dir) = std::env::var_os("XDG_CONFIG_HOME") {
        let config_dir = PathBuf::from(config_dir);
        if config_dir.is_absolute() {
            return Some(config_dir);
        }
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
        .map(|home| home.join(".config"))
}

#[cfg(target_os = "linux")]
fn linux_systemd_unit_path(config_dir: &Path) -> PathBuf {
    config_dir
        .join("systemd/user")
        .join(LINUX_SYSTEMD_UNIT_FILE_NAME)
}

#[cfg(target_os = "linux")]
fn linux_xdg_entry_path(config_dir: &Path) -> PathBuf {
    config_dir
        .join("autostart")
        .join(LINUX_XDG_AUTOSTART_FILE_NAME)
}

#[cfg(target_os = "linux")]
fn linux_desktop_entry_text(executable: &Path) -> Result<String, String> {
    let executable = executable
        .to_str()
        .filter(|path| !path.chars().any(char::is_control))
        .ok_or_else(|| "start-at-login-unavailable".to_owned())?;
    let mut escaped_executable = String::new();
    for character in executable.chars() {
        match character {
            '\\' | '"' | '`' | '$' => {
                escaped_executable.push('\\');
                escaped_executable.push(character);
            }
            '%' => escaped_executable.push_str("%%"),
            _ => escaped_executable.push(character),
        }
    }
    Ok(format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.0\n\
         Name=Trace Commons\n\
         Comment=Review and contribute coding sessions\n\
         Exec=\"{escaped_executable}\"\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n\
         X-TraceCommons-Autostart-Managed=true\n"
    ))
}

#[cfg(target_os = "windows")]
fn windows_login_item_status() -> i32 {
    use std::os::windows::ffi::OsStrExt;

    let Ok(key) = CurrentUserKey::open(WINDOWS_KEY_QUERY_VALUE) else {
        return -1;
    };
    let sub_key = windows_wide(WINDOWS_RUN_KEY);
    let value_name = windows_wide(WINDOWS_RUN_VALUE);
    let mut value_type = 0;
    let mut byte_len = 0;
    // SAFETY: registry strings are NUL-terminated; a null data buffer with a
    // zero length asks RegGetValueW for the required buffer size.
    let status = unsafe {
        RegGetValueW(
            key.0,
            sub_key.as_ptr(),
            value_name.as_ptr(),
            WINDOWS_RRF_RT_REG_SZ,
            &mut value_type,
            std::ptr::null_mut(),
            &mut byte_len,
        )
    };
    if status == WINDOWS_ERROR_FILE_NOT_FOUND {
        return 0;
    }
    if status != 0 || value_type != WINDOWS_REG_SZ || byte_len < 2 || byte_len % 2 != 0 {
        return -1;
    }

    let mut registered = vec![0u16; (byte_len / 2) as usize];
    // SAFETY: `registered` is writable for `byte_len` bytes as requested by
    // the successful sizing call above.
    let status = unsafe {
        RegGetValueW(
            key.0,
            sub_key.as_ptr(),
            value_name.as_ptr(),
            WINDOWS_RRF_RT_REG_SZ,
            &mut value_type,
            registered.as_mut_ptr().cast(),
            &mut byte_len,
        )
    };
    if status != 0 || value_type != WINDOWS_REG_SZ {
        return -1;
    }
    while registered.last() == Some(&0) {
        registered.pop();
    }

    let Ok(executable) = std::env::current_exe() else {
        return -1;
    };
    let mut expected = vec![b'"' as u16];
    expected.extend(executable.as_os_str().encode_wide());
    expected.extend([b'"' as u16, 0]);
    while expected.last() == Some(&0) {
        expected.pop();
    }

    if registered == expected { 1 } else { 3 }
}

#[cfg(target_os = "windows")]
fn windows_set_login_item(enabled: bool) -> i32 {
    use std::os::windows::ffi::OsStrExt;

    match (enabled, windows_login_item_status()) {
        (true, 1) | (false, 0) => return if enabled { 1 } else { 0 },
        (true, 0) | (false, 1) => {}
        // Preserve any value this adapter cannot identify as its own.
        (_, _) => return -1,
    }

    let Ok(key) = CurrentUserKey::open(WINDOWS_KEY_SET_VALUE) else {
        return -1;
    };
    let sub_key = windows_wide(WINDOWS_RUN_KEY);
    let value_name = windows_wide(WINDOWS_RUN_VALUE);
    if enabled {
        let Ok(executable) = std::env::current_exe() else {
            return -1;
        };
        let mut command = vec![b'"' as u16];
        command.extend(executable.as_os_str().encode_wide());
        command.extend([b'"' as u16, 0]);
        let Some(byte_len) = command
            .len()
            .checked_mul(std::mem::size_of::<u16>())
            .and_then(|size| u32::try_from(size).ok())
        else {
            return -1;
        };
        // SAFETY: the key is open for writes, and `command` is a NUL-terminated
        // UTF-16 string whose byte length is passed exactly.
        let status = unsafe {
            RegSetKeyValueW(
                key.0,
                sub_key.as_ptr(),
                value_name.as_ptr(),
                WINDOWS_REG_SZ,
                command.as_ptr().cast(),
                byte_len,
            )
        };
        if status != 0 {
            return -1;
        }
        windows_login_item_status()
    } else {
        // SAFETY: the key is open for writes, and both names are NUL-terminated.
        let status = unsafe { RegDeleteKeyValueW(key.0, sub_key.as_ptr(), value_name.as_ptr()) };
        if status == 0 || status == WINDOWS_ERROR_FILE_NOT_FOUND {
            0
        } else {
            -1
        }
    }
}

#[cfg(target_os = "windows")]
fn windows_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::{
        login_item_change_succeeded, login_item_state, notification_can_post, notification_state,
        set_review_route, tc_notification_review_requested,
    };

    // One test owns the process-wide route, so parallel tests cannot race
    // over which closure is registered.
    #[test]
    fn a_notification_review_click_is_routed_in_process() {
        // Before configuration there is nothing to route to, and the click
        // must be a no-op rather than a crash on the notification thread.
        tc_notification_review_requested();

        let calls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        set_review_route(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        });
        tc_notification_review_requested();
        tc_notification_review_requested();
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        // A panic must never unwind across the Objective-C frame that
        // called in.
        set_review_route(|| panic!("route failed"));
        tc_notification_review_requested();

        // Re-registering replaces the route rather than stacking it.
        let replaced = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&replaced);
        set_review_route(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        });
        tc_notification_review_requested();
        assert_eq!(replaced.load(Ordering::SeqCst), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn native_state_mapping_is_explicit() {
        assert_eq!(notification_state(0), "requires_approval");
        assert_eq!(notification_state(1), "denied");
        assert_eq!(notification_state(2), "available");
        assert_eq!(notification_state(-1), "unknown");
        assert!(!notification_can_post(0));
        assert!(!notification_can_post(1));
        assert!(notification_can_post(2));
        assert!(notification_can_post(3));
        assert_eq!(login_item_state(0), "not_registered");
        assert_eq!(login_item_state(1), "enabled");
        assert_eq!(login_item_state(2), "requires_approval");
        assert_eq!(login_item_state(3), "not_found");
        assert_eq!(login_item_state(4), "managed");
        assert_eq!(login_item_state(-1), "unknown");
        assert!(login_item_change_succeeded(1, true));
        assert!(login_item_change_succeeded(2, true));
        assert!(!login_item_change_succeeded(3, true));
        assert!(login_item_change_succeeded(0, false));
        assert!(login_item_change_succeeded(3, false));
        assert!(!login_item_change_succeeded(1, false));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_startup_entry_round_trips_without_clobbering_existing_files() {
        use super::{linux_login_item_status_in, linux_set_login_item_in, linux_xdg_entry_path};

        let config_dir = ScratchConfigDir::new();
        let executable = config_dir
            .path()
            .join("Trace Commons")
            .join("trace-commons");
        assert_eq!(
            linux_login_item_status_in(config_dir.path(), &executable),
            0
        );

        linux_set_login_item_in(config_dir.path(), &executable, true).unwrap();
        assert_eq!(
            linux_login_item_status_in(config_dir.path(), &executable),
            1
        );
        linux_set_login_item_in(config_dir.path(), &executable, true).unwrap();
        linux_set_login_item_in(config_dir.path(), &executable, false).unwrap();
        assert_eq!(
            linux_login_item_status_in(config_dir.path(), &executable),
            0
        );

        let foreign_entry = linux_xdg_entry_path(config_dir.path());
        std::fs::create_dir_all(foreign_entry.parent().unwrap()).unwrap();
        std::fs::write(&foreign_entry, "user-owned desktop entry\n").unwrap();
        let before = std::fs::read(&foreign_entry).unwrap();
        assert!(linux_set_login_item_in(config_dir.path(), &executable, true).is_err());
        assert!(linux_set_login_item_in(config_dir.path(), &executable, false).is_err());
        assert_eq!(std::fs::read(&foreign_entry).unwrap(), before);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_startup_entry_quotes_and_escapes_executable_path() {
        use super::linux_desktop_entry_text;
        use std::path::Path;

        let executable = Path::new(r#"/tmp/Trace Commons/$cash/%quoted"/slash\`"#);
        let entry = linux_desktop_entry_text(executable).unwrap();

        assert!(entry.contains(r#"Exec="/tmp/Trace Commons/\$cash/%%quoted\"/slash\\\`""#));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_systemd_unit_takes_precedence_over_xdg_autostart() {
        use super::{
            linux_login_item_status_in, linux_set_login_item_in, linux_systemd_unit_path,
            linux_xdg_entry_path,
        };

        let config_dir = ScratchConfigDir::new();
        let executable = config_dir.path().join("trace-commons");
        let unit = linux_systemd_unit_path(config_dir.path());
        std::fs::create_dir_all(unit.parent().unwrap()).unwrap();
        std::fs::write(
            &unit,
            "[Service]\nExecStart=/usr/bin/trace-commons-contributor\n",
        )
        .unwrap();
        let xdg_entry = linux_xdg_entry_path(config_dir.path());
        std::fs::create_dir_all(xdg_entry.parent().unwrap()).unwrap();
        std::fs::write(&xdg_entry, "pre-existing entry\n").unwrap();
        let before = std::fs::read(&xdg_entry).unwrap();

        assert_eq!(
            linux_login_item_status_in(config_dir.path(), &executable),
            4
        );
        linux_set_login_item_in(config_dir.path(), &executable, true).unwrap();
        assert!(linux_set_login_item_in(config_dir.path(), &executable, false).is_err());
        assert_eq!(std::fs::read(&xdg_entry).unwrap(), before);
    }

    #[cfg(target_os = "linux")]
    struct ScratchConfigDir(std::path::PathBuf);

    #[cfg(target_os = "linux")]
    impl ScratchConfigDir {
        fn new() -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "trace-commons-tauri-native-test-{}-{unique}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    #[cfg(target_os = "linux")]
    impl Drop for ScratchConfigDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
