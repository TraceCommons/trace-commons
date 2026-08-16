//! Pins the invariants of the release path. These files are shell scripts and
//! workflow YAML, so the assertions are deliberately textual: a YAML parser
//! would mean a new dependency, and the properties worth pinning here (an env
//! contract, a mandatory flag, an exclusion) are visible in the text.
//!
//! What this file CANNOT prove: that signing, notarization, or a flatpak build
//! actually work. Only a real run against real credentials shows that. See
//! `docs/release-runbook.md` for those gates.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

#[test]
fn info_plist_script_injects_the_version_it_is_given() {
    let script = repo_root().join("macos/scripts/info-plist.sh");
    let output = Command::new("bash")
        .arg(&script)
        .args(["0.4.2", "17"])
        .output()
        .expect("failed to run info-plist.sh");
    assert!(
        output.status.success(),
        "info-plist.sh failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let plist = String::from_utf8_lossy(&output.stdout);

    assert!(
        plist.contains("<key>CFBundleShortVersionString</key><string>0.4.2</string>"),
        "the short version was not injected:\n{plist}"
    );
    assert!(
        plist.contains("<key>CFBundleVersion</key><string>17</string>"),
        "the build version was not injected:\n{plist}"
    );
    // A release must never ship the placeholder the old heredoc hardcoded.
    assert!(
        !plist.contains("0.1.0"),
        "the hardcoded 0.1.0 is still present:\n{plist}"
    );
    // Regressions here are silent and severe: without LSUIElement the menu-bar
    // app grows a Dock icon, and without the bundle id notifications break.
    assert!(
        plist.contains("<key>LSUIElement</key><true/>"),
        "LSUIElement lost"
    );
    assert!(
        plist.contains("<key>CFBundleIdentifier</key><string>ai.tracecommons.shell</string>"),
        "bundle id lost"
    );
}

#[test]
fn bundle_script_passes_its_version_through_to_the_plist() {
    let script = read("macos/scripts/make-app-bundle.sh");
    assert!(
        script.contains("info-plist.sh"),
        "make-app-bundle.sh must delegate to info-plist.sh rather than \
         carrying its own heredoc, or the two will drift"
    );
    assert!(
        !script.contains("CFBundleShortVersionString"),
        "the plist heredoc is still inline in make-app-bundle.sh"
    );
}

#[test]
fn swift_manifest_takes_the_library_path_from_the_environment() {
    let manifest = read("macos/Package.swift");
    assert!(
        manifest.contains("environment[\"TC_FFI_LIB_DIR\"]"),
        "Package.swift must read the FFI library search path from \
         TC_FFI_LIB_DIR. Hardcoding ../target/debug makes a release build \
         link against a directory that does not exist in CI."
    );
    // The env var is read once and reused; a literal debug path left in a
    // linkerSettings block would silently win for that target. Check within
    // .unsafeFlags blocks to avoid false positives from comments or later content.
    let unsafe_flags_count = manifest.matches(".unsafeFlags([").count();
    assert!(
        unsafe_flags_count >= 2,
        "unsafeFlags spelling changed; the hardcoded-path scan is now vacuous"
    );
    let hardcoded_in_linker_settings = manifest.split(".unsafeFlags([").skip(1).any(|section| {
        // Each section runs from .unsafeFlags([ to the next ]) that closes it
        section
            .split("])")
            .next()
            .map_or(false, |flags| flags.contains("../target/debug"))
    });
    assert!(
        !hardcoded_in_linker_settings,
        "a linkerSettings block still hardcodes ../target/debug"
    );
}

#[test]
fn bundle_script_exports_the_library_path_and_can_skip_adhoc_signing() {
    let script = read("macos/scripts/make-app-bundle.sh");
    assert!(
        script.contains("export TC_FFI_LIB_DIR="),
        "make-app-bundle.sh must export TC_FFI_LIB_DIR so swift build links \
         against target/$CONFIG"
    );
    assert!(
        script.contains("TC_SKIP_ADHOC_SIGN:-"),
        "the release path must be able to skip the ad-hoc signature rather \
         than have make-release-dmg.sh re-sign over it"
    );
    assert!(
        script.contains("TC_SKIP_ADHOC_SIGN:-0}\" != \"1\""),
        "the guard must skip codesigning when TC_SKIP_ADHOC_SIGN is set to 1; \
         inverting the condition would ad-hoc-sign every release build"
    );
}

#[test]
fn release_dmg_notarizes_with_an_api_key_not_a_password() {
    let script = read("macos/scripts/make-release-dmg.sh");
    // Filter out comments to assert against code, not prose.
    let code = script
        .lines()
        .filter(|line| line.trim_start().is_empty() || !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");

    // API key credentials must be required and actually used.
    for required in [
        "MACOS_NOTARY_ASC_KEY_P8_BASE64",
        "MACOS_NOTARY_ASC_KEY_ID",
        "MACOS_NOTARY_ASC_ISSUER_ID",
    ] {
        assert!(
            code.contains(required),
            "make-release-dmg.sh must require {required} in executable code"
        );
    }

    // The API key must actually be passed to notarytool, not just required.
    assert!(
        code.contains("--key \"$WORK/notary.p8\""),
        "notarytool must be called with --key pointing to the decoded API key file"
    );
    assert!(
        code.contains("--key-id"),
        "notarytool must be called with --key-id"
    );
    assert!(
        code.contains("--issuer"),
        "notarytool must be called with --issuer"
    );

    // Old Apple ID + password credentials are completely gone from code.
    for gone in ["MACOS_NOTARY_APPLE_ID", "MACOS_NOTARY_PASSWORD"] {
        assert!(
            !code.contains(gone),
            "{gone} is still in executable code; the Apple ID + app-specific password \
             path was replaced by the ASC API key"
        );
    }
    assert!(
        !code.contains("store-credentials"),
        "notarytool store-credentials is no longer in executable code"
    );

    // Hardened runtime and stapling are still present in code.
    assert!(
        code.contains("--options runtime"),
        "hardened runtime is required for notarization"
    );
    assert!(
        code.contains("stapler staple"),
        "an unstapled DMG fails for a user who is offline"
    );

    // Version parameters must be required, not defaulted.
    assert!(
        code.contains("${1:?"),
        "SHORT_VERSION must be required with ${{1:?...}}"
    );
    assert!(
        code.contains("${2:?"),
        "BUILD_VERSION must be required with ${{2:?...}}"
    );
}
