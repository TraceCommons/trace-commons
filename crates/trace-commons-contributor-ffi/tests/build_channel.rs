//! `tc_build_channel_json` and `tc_channel_mismatch_line`: the macOS app
//! reads its channel identity from the dylib it links, never from a copy of
//! its own (staging design, sections 4.1 and 4.3).
//!
//! Every assertion compares against `trace_commons_contributor::channel`, so
//! the same tests hold in a default build and under the contributor's
//! `near-ai-staging` feature.

use std::collections::BTreeSet;
use std::ffi::{CStr, c_char};

use trace_commons_contributor::{channel, health_copy};
use trace_commons_contributor_ffi::{
    tc_build_channel_json, tc_channel_mismatch_line, tc_channel_window_label, tc_resolve_state_dir,
    tc_state_dir_override_refused_line, tc_string_free,
};

fn take_owned(out: *mut c_char) -> String {
    assert!(!out.is_null(), "the export returned NULL");
    let s = unsafe { CStr::from_ptr(out) }
        .to_string_lossy()
        .into_owned();
    unsafe { tc_string_free(out) };
    s
}

fn channel_json() -> serde_json::Map<String, serde_json::Value> {
    let value: serde_json::Value =
        serde_json::from_str(&take_owned(tc_build_channel_json())).expect("the export is JSON");
    value.as_object().expect("the export is an object").clone()
}

#[test]
fn the_channel_json_matches_the_channel_module() {
    let json = channel_json();
    let field = |name: &str| {
        json.get(name)
            .and_then(|value| value.as_str())
            .unwrap_or_else(|| panic!("`{name}` is a string"))
            .to_string()
    };
    assert_eq!(field("channel"), channel::CHANNEL);
    assert_eq!(field("state_dir_name"), channel::STATE_DIR_NAME);
    assert_eq!(field("state_dir_env"), channel::STATE_DIR_ENV);
    assert_eq!(field("passkey_rp_id"), channel::PASSKEY_RP_ID);
    assert_eq!(field("url_scheme"), channel::URL_SCHEME);
    assert_eq!(
        field("near_ai_dashboard_origin"),
        channel::NEAR_AI_DASHBOARD_ORIGIN
    );
    assert_eq!(
        channel::CHANNEL == "staging",
        channel::near_ai_staging(),
        "the channel name follows the feature"
    );
}

/// The sign-in providers the app offers come from the core, in its order:
/// production's NEAR wallet first, staging's NEAR AI Google login alone.
#[test]
fn the_channel_json_names_the_sign_in_providers_the_daemon_accepts() {
    let json = channel_json();
    let providers: Vec<&str> = json
        .get("near_ai_sign_in_providers")
        .and_then(|value| value.as_array())
        .expect("`near_ai_sign_in_providers` is an array")
        .iter()
        .map(|value| value.as_str().expect("each provider is a string"))
        .collect();
    assert_eq!(providers, channel::NEAR_AI_SIGN_IN_PROVIDERS);
    if channel::near_ai_staging() {
        assert_eq!(providers, ["google"]);
    } else {
        assert_eq!(providers, ["near", "github", "google"]);
    }
}

#[test]
fn the_channel_json_carries_no_key() {
    let json = channel_json();
    // Exactly these fields: anything else the app needs is a reviewed
    // addition, so a key, a secret or a keychain name cannot ride along.
    let keys: BTreeSet<&str> = json.keys().map(String::as_str).collect();
    let expected: BTreeSet<&str> = [
        "channel",
        "state_dir_name",
        "state_dir_env",
        "passkey_rp_id",
        "url_scheme",
        "near_ai_dashboard_origin",
        "near_ai_sign_in_providers",
    ]
    .into_iter()
    .collect();
    assert_eq!(keys, expected);

    let keychain = channel::KEYCHAIN;
    let withheld = [
        channel::KEYCHAIN_ACCESS_GROUP,
        keychain.cloud_service,
        keychain.account_service,
        keychain.insights_digest_key_service,
        keychain.claude_usage_service,
    ];
    // Every field is a string but the provider list, which holds strings.
    let values = json.values().flat_map(|value| match value.as_array() {
        Some(list) => list.iter().collect::<Vec<_>>(),
        None => vec![value],
    });
    for value in values {
        let value = value.as_str().expect("every field is a string");
        assert!(!value.is_empty(), "no field is empty");
        for name in withheld {
            assert!(
                !value.contains(name),
                "the channel JSON names a keychain identity"
            );
        }
    }
}

#[test]
fn the_channel_mismatch_line_is_the_core_sentence() {
    assert_eq!(
        take_owned(tc_channel_mismatch_line()),
        health_copy::CHANNEL_MISMATCH
    );
    // Copy guide: say what is happening to the contributor's data.
    assert!(health_copy::CHANNEL_MISMATCH.contains("Nothing is being watched or sent"));
}

#[test]
fn the_window_label_is_the_core_word_on_staging_and_empty_on_production() {
    let label = take_owned(tc_channel_window_label());
    assert_eq!(label, health_copy::window_label().unwrap_or(""));
    assert_eq!(
        label.is_empty(),
        channel::CHANNEL == "production",
        "only a staging build shows a label"
    );
}

/// `tc_resolve_state_dir` with `explicit` and `override_dir` as C strings
/// (`None` is NULL): the owned path, or the owned `*err` label.
fn resolve_state_dir(explicit: Option<&str>, override_dir: Option<&str>) -> Result<String, String> {
    let explicit = explicit.map(|s| std::ffi::CString::new(s).unwrap());
    let override_dir = override_dir.map(|s| std::ffi::CString::new(s).unwrap());
    let ptr = |s: &Option<std::ffi::CString>| s.as_ref().map_or(std::ptr::null(), |s| s.as_ptr());
    let mut err: *mut c_char = std::ptr::null_mut();
    let out = unsafe { tc_resolve_state_dir(ptr(&explicit), ptr(&override_dir), &mut err) };
    if out.is_null() {
        Err(take_owned(err))
    } else {
        assert!(err.is_null(), "a success sets no error");
        Ok(take_owned(out))
    }
}

/// The other channel's default directory on this platform: beside this
/// channel's default, under the other channel's name. On macOS that is
/// `~/Library/Application Support/<name>`, where the app's Swift code puts
/// it.
fn other_channel_default() -> String {
    let own = resolve_state_dir(None, None).expect("a platform default");
    let (parent, name) = own
        .rsplit_once(std::path::MAIN_SEPARATOR)
        .expect("a parent");
    assert_eq!(name, channel::STATE_DIR_NAME);
    format!(
        "{parent}{}{}",
        std::path::MAIN_SEPARATOR,
        channel::OTHER_STATE_DIR_NAME
    )
}

#[test]
fn an_override_naming_the_other_channel_directory_is_refused_without_a_path() {
    let other = other_channel_default();
    let nested = format!("{other}{}nested", std::path::MAIN_SEPARATOR);
    for candidate in [other.clone(), nested] {
        assert_eq!(
            resolve_state_dir(None, Some(&candidate)),
            Err("state-dir-other-channel".to_string())
        );
    }
}

#[cfg(any(target_os = "macos", windows))]
#[test]
fn a_case_variant_of_the_other_channel_directory_is_refused() {
    let other = other_channel_default();
    let (parent, name) = other
        .rsplit_once(std::path::MAIN_SEPARATOR)
        .expect("a parent");
    let upper = format!(
        "{parent}{}{}",
        std::path::MAIN_SEPARATOR,
        name.to_ascii_uppercase()
    );
    assert_eq!(
        resolve_state_dir(None, Some(&upper)),
        Err("state-dir-other-channel".to_string())
    );
}

#[test]
fn the_state_dir_precedence_is_explicit_then_override_then_default() {
    let other = other_channel_default();
    // An explicit directory is the caller's own and is not checked.
    assert_eq!(
        resolve_state_dir(Some(&other), Some("/tmp/override")),
        Ok(other.clone())
    );
    assert_eq!(
        resolve_state_dir(None, Some("/tmp/tc-own-override")),
        Ok("/tmp/tc-own-override".to_string())
    );
    let default = resolve_state_dir(None, None).expect("a platform default");
    assert!(
        default.ends_with(channel::STATE_DIR_NAME),
        "the default carries this channel's name"
    );
    // An empty string is unset, never a directory anyone chose.
    assert_eq!(resolve_state_dir(Some(""), Some("")), Ok(default));
}

#[test]
fn a_non_utf8_state_dir_argument_is_refused_with_a_fixed_label() {
    let bytes = [0xff_u8, 0xfe, 0];
    let mut err: *mut c_char = std::ptr::null_mut();
    let out = unsafe { tc_resolve_state_dir(std::ptr::null(), bytes.as_ptr().cast(), &mut err) };
    assert!(out.is_null());
    assert_eq!(take_owned(err), "state-dir-unresolved");
}

#[test]
fn the_override_refused_line_is_the_core_sentence_and_names_no_path() {
    let line = take_owned(tc_state_dir_override_refused_line());
    assert_eq!(line, health_copy::STATE_DIR_OVERRIDE_REFUSED);
    // Copy guide: say what is happening to the contributor's data.
    assert!(line.contains("Nothing is being watched or sent"));
    for name in [channel::STATE_DIR_NAME, channel::OTHER_STATE_DIR_NAME] {
        assert!(!line.contains(name), "the sentence names no directory");
    }
}
