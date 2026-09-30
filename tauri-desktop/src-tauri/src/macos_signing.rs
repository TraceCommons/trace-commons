//! Drift checks for the macOS release signing inputs.
//!
//! The contributor crate stores Private AI credentials in the data-protection
//! keychain under one access group, `KXSWJN7WY8.ai.tracecommons.shell`. The
//! native shell and this app each request it in their own entitlements file,
//! and each is signed under its own App ID. Those are four strings in three
//! files, edited independently, and a mismatch in any of them is not a
//! degraded feature: a wrong group makes every credential operation report
//! `storage_unentitled`, and a wrong application identifier gets the app
//! killed at exec. Nothing else compares them before a release.

const TAURI_ENTITLEMENTS: &str = include_str!("../entitlements.plist");
const NATIVE_ENTITLEMENTS: &str = include_str!("../../../macos/entitlements.plist");
const RELEASE_CONFIG: &str = include_str!("../tauri.release.conf.json");

const TEAM_ID: &str = "KXSWJN7WY8";
const ACCESS_GROUP: &str = "KXSWJN7WY8.ai.tracecommons.shell";

/// The body of the `<array>` that follows `<key>{key}</key>`.
fn array_after_key<'a>(plist: &'a str, key: &str) -> &'a str {
    let marker = format!("<key>{key}</key>");
    let rest = &plist[plist
        .find(&marker)
        .unwrap_or_else(|| panic!("{key} is declared"))
        + marker.len()..];
    let start = rest.find("<array>").expect("an array follows the key") + "<array>".len();
    let end = rest.find("</array>").expect("the array is closed");
    rest[start..end].trim()
}

/// The `<string>` that follows `<key>{key}</key>`.
fn string_after_key<'a>(plist: &'a str, key: &str) -> &'a str {
    let marker = format!("<key>{key}</key>");
    let rest = &plist[plist
        .find(&marker)
        .unwrap_or_else(|| panic!("{key} is declared"))
        + marker.len()..];
    let start = rest.find("<string>").expect("a string follows the key") + "<string>".len();
    let end = rest.find("</string>").expect("the string is closed");
    &rest[start..end]
}

#[test]
fn both_apps_request_the_same_keychain_access_group() {
    let tauri = array_after_key(TAURI_ENTITLEMENTS, "keychain-access-groups");
    let native = array_after_key(NATIVE_ENTITLEMENTS, "keychain-access-groups");
    assert_eq!(tauri, format!("<string>{ACCESS_GROUP}</string>"));
    assert_eq!(
        tauri, native,
        "a credential one app stores must be readable by the other"
    );
}

#[test]
fn the_application_identifier_is_the_release_bundle_id() {
    let config: serde_json::Value =
        serde_json::from_str(RELEASE_CONFIG).expect("release config is JSON");
    let identifier = config["identifier"].as_str().expect("release identifier");
    assert_eq!(identifier, "ai.tracecommons.desktop");
    assert_eq!(
        string_after_key(TAURI_ENTITLEMENTS, "com.apple.application-identifier"),
        format!("{TEAM_ID}.{identifier}"),
    );
    assert_eq!(
        string_after_key(TAURI_ENTITLEMENTS, "com.apple.developer.team-identifier"),
        TEAM_ID,
    );
}

#[test]
fn the_release_config_signs_with_the_entitlements_and_embeds_the_profile() {
    let config: serde_json::Value =
        serde_json::from_str(RELEASE_CONFIG).expect("release config is JSON");
    let mac = &config["bundle"]["macOS"];
    assert_eq!(mac["hardenedRuntime"], true);
    assert_eq!(mac["entitlements"], "entitlements.plist");
    assert_eq!(
        mac["files"]["embedded.provisionprofile"],
        "TraceCommonsDesktop-DeveloperID.provisionprofile"
    );
}
