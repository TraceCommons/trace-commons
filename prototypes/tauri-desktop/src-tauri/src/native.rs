use std::ffi::CString;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn tc_macos_notification_configure() -> i32;
    fn tc_macos_notification_status() -> i32;
    fn tc_macos_request_notification_permission() -> i32;
    fn tc_macos_post_digest(body: *const std::ffi::c_char) -> i32;
    fn tc_macos_login_item_status() -> i32;
    fn tc_macos_set_login_item(enabled: i32) -> i32;
}

pub(crate) fn configure_notifications() {
    #[cfg(target_os = "macos")]
    // The native bridge returns a fixed status only. There is no user-facing
    // failure at launch: an unsigned/dev binary may not have a notification
    // center, while the packaged app will configure its category here.
    unsafe {
        let _ = tc_macos_notification_configure();
    }
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
        return Ok(notification_capability());
    }
    #[cfg(not(target_os = "macos"))]
    Err("notification-permission-unavailable".to_owned())
}

pub(crate) fn login_item_capability() -> serde_json::Value {
    #[cfg(target_os = "macos")]
    let status = unsafe { tc_macos_login_item_status() };
    #[cfg(not(target_os = "macos"))]
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
        return Ok(login_item_capability());
    }
    #[cfg(not(target_os = "macos"))]
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
        return (result == 0)
            .then_some(())
            .ok_or_else(|| "notification-post-failed".to_owned());
    }
    #[cfg(not(target_os = "macos"))]
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

fn notification_can_post(status: i32) -> bool {
    matches!(status, 2 | 3)
}

fn login_item_state(status: i32) -> &'static str {
    match status {
        0 => "not_registered",
        1 => "enabled",
        2 => "requires_approval",
        3 => "not_found",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::{login_item_state, notification_can_post, notification_state};

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
        assert_eq!(login_item_state(-1), "unknown");
    }
}
