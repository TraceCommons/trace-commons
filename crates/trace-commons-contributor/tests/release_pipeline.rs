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

#[test]
fn release_apps_workflow_is_tag_driven_and_per_platform_runnable() {
    let workflow = read(".github/workflows/release-apps.yml");
    assert!(workflow.contains("app-v*"), "must trigger on app-v* tags");
    assert!(
        workflow.contains("workflow_dispatch"),
        "one platform must be re-runnable without cutting a tag"
    );
    // Independent jobs, not matrix legs: the packaging steps share nothing,
    // and one platform failing must not block the others.
    for job in ["  macos:", "  windows:", "  linux-flatpak:"] {
        assert!(workflow.contains(job), "missing job {job}");
    }
}

/// dtolnay/rust-toolchain is pinned to a commit SHA of its master branch
/// (not a `@stable`/`@1.92`-style ref), so it cannot infer the toolchain
/// from the ref name and `toolchain:` becomes a required input. Plain
/// string counting, not a YAML parser: every `dtolnay/rust-toolchain`
/// usage must be matched by a `toolchain: "` input somewhere in the file.
#[test]
fn every_rust_toolchain_usage_pins_a_toolchain_input() {
    for path in [
        ".github/workflows/release-apps.yml",
        ".github/workflows/release-contributor.yml",
    ] {
        let workflow = read(path);
        let uses_count = workflow.matches("dtolnay/rust-toolchain@").count();
        let toolchain_count = workflow.matches("toolchain: \"").count();
        assert!(
            uses_count > 0,
            "{path}: expected at least one dtolnay/rust-toolchain usage"
        );
        assert_eq!(
            uses_count, toolchain_count,
            "{path}: every dtolnay/rust-toolchain usage must carry a \
             `toolchain:` input -- pinned to a commit SHA, the action \
             cannot infer the toolchain from the ref name"
        );
    }
}

/// Both workflows sign Windows binaries with the same duplicated dlib block,
/// so both must be pinned. Reading only one leaves the other free to drop the
/// timestamp -- and an untimestamped Trusted Signing signature keeps validating
/// for about three days, so no same-day run would catch it.
#[test]
fn windows_signing_is_timestamped() {
    for path in [
        ".github/workflows/release-apps.yml",
        ".github/workflows/release-contributor.yml",
    ] {
        assert_windows_signing_is_hardened(path);
    }
}

fn assert_windows_signing_is_hardened(path: &str) {
    let workflow = read(path);
    // Microsoft's dlib driven by signtool, NOT the marketplace action -- so the
    // client can be verified by content before it runs in a job that holds
    // signing authority.
    assert!(
        workflow.contains("Azure.CodeSigning.Dlib.dll"),
        "{path}: Windows signing drives Microsoft's Trusted Signing dlib via signtool"
    );
    assert!(
        !workflow.contains("azure/trusted-signing-action"),
        "{path}: the marketplace action was deliberately replaced by the SHA-verified \
         dlib; reintroducing it drops the content check"
    );
    assert!(
        workflow.contains("TRUSTED_SIGNING_CLIENT_SHA256")
            && workflow.contains("Refusing to expand a potentially tampered"),
        "{path}: the signing client must be verified by SHA-256 and fail closed before \
         extraction"
    );
    // Trusted Signing certificates are valid for roughly three days. Without
    // an RFC3161 countersignature the signature stops validating days after
    // release -- a failure no same-day test would catch.
    assert!(
        workflow.contains("/tr http://timestamp.acs.microsoft.com"),
        "{path}: every sign invocation needs an RFC3161 timestamp server: Trusted \
         Signing certificates carry ~3-day validity, so the countersignature \
         is the only reason a signature outlives them"
    );
    assert!(
        workflow.contains("/td SHA256"),
        "{path}: the timestamp digest algorithm must be pinned alongside /tr"
    );
    assert!(
        workflow.contains("signtool") || workflow.contains("Get-AuthenticodeSignature"),
        "{path}: the signature must be verified in the job, not assumed"
    );
}

#[test]
fn flatpak_manifest_has_its_vendored_sources_enabled() {
    let manifest =
        read("crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.yml");
    assert!(
        !manifest.contains("only-arches: []"),
        "the cargo-sources.json entry is still disabled, so the \
         network-sandboxed cargo build cannot resolve any crate"
    );
    let sources =
        repo_root().join("crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json");
    assert!(
        sources.exists(),
        "cargo-sources.json must be generated and committed; \
         `cargo --offline build` has no crates without it"
    );
    // The confinement argument only holds if the grants stay narrow.
    assert!(
        manifest.contains("--filesystem=~/.claude/projects:ro")
            && manifest.contains("--filesystem=~/.codex/sessions:ro"),
        "the two read-only session roots must remain the only filesystem grants"
    );
    assert!(
        !manifest.contains("--filesystem=home"),
        "a blanket home grant defeats the point of shipping this confined"
    );
    assert_eq!(
        manifest.matches("--filesystem=").count(),
        2,
        "the two read-only session roots must be the ONLY filesystem grants; \
         a third would widen what a transcript-reading app can reach"
    );
}

#[test]
fn cargo_sources_json_looks_like_a_real_generated_source_list() {
    // Plain std::fs plus a manual scan, deliberately: a JSON dependency for
    // one test is not worth it, and this is only meant to catch the file
    // being truncated, replaced with `{}`, or hand-edited into something
    // without checksums -- not to catch drift against Cargo.lock. This scan
    // is coupled to the generator's current output shape: a `type: git`
    // dependency (if the GTK crate ever gained one) emits a url + commit
    // with no sha256, which would fail this url-count == sha256-count check
    // on a perfectly correct file. If that day comes, compare against
    // `"type": "archive"` occurrences instead.
    let sources = read("crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json");
    let trimmed = sources.trim();
    assert!(
        trimmed.starts_with('[') && trimmed.ends_with(']'),
        "cargo-sources.json must be a JSON array, as flatpak-cargo-generator.py \
         produces; got something else entirely"
    );
    let url_count = sources.matches("\"url\"").count();
    let sha256_count = sources.matches("\"sha256\"").count();
    assert!(
        url_count > 0,
        "cargo-sources.json is empty or missing url entries; \
         it looks truncated or hand-edited"
    );
    assert_eq!(
        url_count, sha256_count,
        "every source entry must carry a sha256 alongside its url; a \
         hand-edited or corrupted file could drop checksums silently, \
         which is exactly what this manifest's own comment warns against"
    );

    // Cheap half of the drift problem: catch a `cargo update` inside the
    // GTK crate that was never followed by regenerating cargo-sources.json.
    // This does not parse TOML (no new dependency) -- it walks Cargo.lock's
    // `[[package]]` blocks by hand, which is stable enough for this format.
    // It cannot catch every kind of drift (a source removed but a stale
    // entry left behind, for instance), but a registry package in the
    // lockfile with no matching vendor entry is exactly the failure that
    // would otherwise surface 60 minutes into a release job, at the
    // network-sandboxed `cargo --offline build` step.
    let lockfile = read("crates/trace-commons-contributor-gtk/Cargo.lock");
    for block in lockfile.split("[[package]]").skip(1) {
        if !block.contains("source = \"registry+") {
            continue; // path/git dependencies aren't vendored this way
        }
        let name = block
            .lines()
            .find_map(|l| l.trim().strip_prefix("name = \""))
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or_else(|| panic!("package block with no name:\n{block}"));
        let version = block
            .lines()
            .find_map(|l| l.trim().strip_prefix("version = \""))
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or_else(|| panic!("package block with no version:\n{block}"));
        let dest = format!("cargo/vendor/{name}-{version}\"");
        assert!(
            sources.contains(&dest),
            "Cargo.lock has {name} {version} from a registry, but \
             cargo-sources.json has no {dest} entry -- it is stale; \
             regenerate it with flatpak-cargo-generator.py"
        );
    }
}

#[test]
fn contributor_release_notes_do_not_teach_past_gatekeeper() {
    let workflow = read(".github/workflows/release-contributor.yml");
    for stale in [
        "not code-signed or notarized",
        "Signing needs an Apple Developer identity and is not set up yet",
    ] {
        assert!(
            !workflow.contains(stale),
            "the release notes still say {stale:?}, which trains \
             contributors past the warning that should stop a tampered build"
        );
    }
    assert!(
        workflow.contains("notarytool"),
        "the macOS CLI binaries must be notarized"
    );
    // notarytool accepts a disk image, a package, or a zip -- never a bare
    // Mach-O. `ditto -c -k` is what actually produces that zip; checking for
    // the bare substring "zip" would also match "$OUT.zip", "pkgZip", and
    // assorted comments, so it can never fail.
    assert!(
        workflow.contains("ditto -c -k"),
        "a bare binary cannot be submitted for notarization; zip it first"
    );
    assert!(
        workflow.contains("x86_64-pc-windows-msvc"),
        "Windows must be in the release matrix"
    );
    // notarytool's --wait exit status is not documented to be non-zero for a
    // rejected submission, so the workflow must parse the verdict itself
    // rather than trusting the exit code.
    assert!(
        workflow.contains("notary.json") && workflow.contains("Accepted"),
        "notarization must parse the submitted verdict and refuse to publish \
         anything other than 'Accepted'"
    );
}

#[test]
fn contributor_release_notes_do_not_promise_an_unpublished_flatpak() {
    let workflow = read(".github/workflows/release-contributor.yml");
    // The linux-flatpak job (in release-apps.yml) publishes the signed OSTree
    // repo, but only on a tag push of app-v* -- a wholly separate workflow
    // and trigger from this file's contributor-v* releases. Pointing this
    // workflow's release notes at that bucket would promise Linux
    // contributors a channel this workflow itself never fills.
    assert!(
        !workflow.contains("tracecommons-flatpak"),
        "the release-contributor notes must not point at the flatpak bucket; \
         that channel is published by release-apps.yml on a different tag"
    );
    assert!(
        workflow.contains("Verify it against the published"),
        "the Linux binary ships unsigned; the notes must point at the \
         checksum, not at a signed distribution channel that does not exist"
    );
}

#[test]
fn flatpak_repo_is_gpg_signed_before_publication() {
    let script = read("scripts/flatpak/build-and-sign.sh");
    assert!(
        script.contains("build-sign") && script.contains("build-update-repo"),
        "both the commit and the repo summary must be signed; a signed commit \
         under an unsigned summary still lets a repo be rolled back or \
         truncated by whoever serves it"
    );
    assert!(
        script.contains("--gpg-sign"),
        "the OSTree repo must be signed with our key"
    );
    let publish = read("scripts/flatpak/publish-repo.sh");
    assert!(
        publish.contains("GPGKey="),
        "the .flatpakref must embed the public key, or the contributor's \
         first install has nothing to verify against"
    );
}
