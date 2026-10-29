//! `tc_cli_tool_json`: the macOS app's command-line tool row reads its link
//! path, state and privileged scripts from the core (`cli_tool::probe`).
//!
//! Reads `/usr/local/bin` on the test machine (never writes it), so a test
//! asserts only what holds whatever is there.

use std::ffi::{CStr, CString, c_char};

use trace_commons_contributor::{channel, cli_tool};
use trace_commons_contributor_ffi::{tc_cli_tool_json, tc_string_free};

fn take_owned(out: *mut c_char) -> serde_json::Value {
    assert!(!out.is_null(), "the export returned NULL");
    let text = unsafe { CStr::from_ptr(out) }
        .to_string_lossy()
        .into_owned();
    unsafe { tc_string_free(out) };
    serde_json::from_str(&text).expect("the export is JSON")
}

fn status(bundle: Option<&str>) -> serde_json::Value {
    let arg = bundle.map(|path| CString::new(path).unwrap());
    take_owned(unsafe { tc_cli_tool_json(arg.as_ref().map_or(std::ptr::null(), |c| c.as_ptr())) })
}

#[test]
fn no_bundle_is_unavailable_and_offers_nothing() {
    let name = cli_tool::command_name_for(channel::CHANNEL).unwrap();
    for bundle in [
        None,
        Some(""),
        Some("/usr/bin"),
        Some("/nowhere/Near.ai.app"),
    ] {
        let value = status(bundle);
        assert_eq!(value["state"], "unavailable", "{bundle:?}");
        assert_eq!(value["command_name"], name);
        assert_eq!(value["link_path"], format!("/usr/local/bin/{name}"));
        assert!(value["install_script"].is_null());
        assert!(value["uninstall_script"].is_null());
    }
}

#[cfg(unix)]
#[test]
fn a_bundle_with_its_cli_gets_a_state_and_a_quoted_script() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let bundle = root.path().join("Near.ai Staging.app");
    let macos = bundle.join("Contents/MacOS");
    std::fs::create_dir_all(&macos).unwrap();
    let cli = macos.join("near-ai");
    std::fs::write(&cli, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();

    let value = status(Some(bundle.to_str().unwrap()));
    let mut keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "command_name",
            "install_script",
            "link_path",
            "state",
            "uninstall_script"
        ]
    );
    // This fresh bundle is never what an existing link points at, so the
    // state is one of these three whatever the machine has installed.
    let state = value["state"].as_str().unwrap();
    assert!(
        ["not_installed", "stale", "foreign"].contains(&state),
        "{state}"
    );
    if state != "foreign" {
        let script = value["install_script"].as_str().unwrap();
        assert!(script.starts_with("do shell script \""));
        assert!(script.ends_with("\" with administrator privileges"));
        assert!(
            script.contains(&format!("/bin/ln -sfn '{}'", cli.display())),
            "{script}"
        );
    } else {
        assert!(value["install_script"].is_null());
    }
}
