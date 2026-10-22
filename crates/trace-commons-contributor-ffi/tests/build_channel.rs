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
    tc_build_channel_json, tc_channel_mismatch_line, tc_string_free,
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
        channel::CHANNEL == "staging",
        channel::near_ai_staging(),
        "the channel name follows the feature"
    );
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
    ];
    for value in json.values() {
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
