//! Pins the invariants of the release path. These files are shell scripts and
//! workflow YAML, so the assertions are deliberately textual: a YAML parser
//! would mean a new dependency, and the properties worth pinning here (an env
//! contract, a mandatory flag, an exclusion) are visible in the text.
//!
//! What this file CANNOT prove: that signing, notarization, or a flatpak build
//! actually work. Only a real run against real credentials shows that. See
//! `docs/release-runbook.md` for those gates.

use std::path::PathBuf;
// Only the bash-invoking tests below use this, and they are all `cfg(unix)`
// -- the scripts they run are bash describing a macOS bundle. Left ungated,
// the import is unused on Windows, and these tests build under
// `-D warnings`, so that is a build failure rather than a lint.
#[cfg(unix)]
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// Reads a repository file with line endings normalized to LF.
///
/// Git on Windows defaults `core.autocrlf` to true, so a Windows checkout
/// delivers these workflow and script files with CRLF endings -- 1367 pairs
/// in `release-apps.yml` alone. Every assertion below is textual by design
/// (see the module comment), so an unnormalized read makes each one
/// platform-dependent: `contains` of any multi-line fragment stops matching,
/// and byte offsets computed from line lengths drift. Normalizing once here
/// keeps the tests asserting about content rather than about the checkout
/// that produced it.
fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    raw.replace("\r\n", "\n")
}

/// Executes `macos/scripts/info-plist.sh`, so it is unix-only: the script is
/// bash describing a macOS bundle, and Windows has no `bash` on PATH to run
/// it with. Without this gate the test does not skip on Windows, it panics
/// with `NotFound "program not found"` -- a failure about the runner rather
/// than about the plist.
///
/// The gate is per-test rather than on the whole target: most of this file
/// pins Windows release behaviour (`windows_msix_*`,
/// `ci_packages_and_validates_the_windows_app_feed_identity`,
/// `windows_signing_is_timestamped`), and those are exactly the assertions
/// most worth running ON Windows.
#[cfg(unix)]
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
    // This assertion used to run the other way, requiring LSUIElement, because
    // the app was a menu-bar utility and "grows a Dock icon" was the
    // regression. That decision was reversed deliberately: on a notched Mac
    // with a full menu bar the status item is assigned a frame in the dead
    // band past the notch and never drawn, which makes a menu-bar-only app
    // unreachable rather than merely discreet. The Dock icon is now the way
    // in, so LSUIElement returning is the regression.
    assert!(
        !plist.contains("<key>LSUIElement</key>"),
        "LSUIElement is back; the app would lose its Dock icon:\n{plist}"
    );
    // The artwork can sit in Contents/Resources and still not be used:
    // without these keys macOS falls back to the generic application icon,
    // which looks exactly like a build that forgot the artwork.
    //
    // Both are asserted because losing either is a silent downgrade rather
    // than a failure. CFBundleIconName resolves AppIcon out of Assets.car --
    // the macOS 26 icon with the Liquid Glass treatment -- and dropping it
    // quietly demotes the app to the flat legacy icns, which still renders
    // and so would pass any check that only asked whether an icon appeared.
    assert!(
        plist.contains("<key>CFBundleIconName</key><string>AppIcon</string>"),
        "CFBundleIconName lost; the Dock would fall back to the legacy icns:\n{plist}"
    );
    assert!(
        plist.contains("<key>CFBundleIconFile</key><string>AppIcon</string>"),
        "CFBundleIconFile lost; the Dock would show the generic icon:\n{plist}"
    );
    assert!(
        plist.contains("<key>CFBundleIdentifier</key><string>ai.tracecommons.shell</string>"),
        "bundle id lost"
    );
    // A dead deep link is the silent, severe regression this test exists for:
    // nothing crashes and nothing logs, the app simply stops answering invite
    // mail. That is how the gap survived to 0.3.0, with onOpenURL wired and
    // nothing declared.
    //
    // Two assertions, not one. A declaration carrying a renamed or mistyped
    // scheme passes a presence-only check while leaving the link just as dead.
    //
    // The limit is worth stating: this proves the plist DECLARES the scheme.
    // It cannot prove LaunchServices ROUTES it, which depends on the
    // registration database and on which bundle wins when several claim the
    // same identifier. That half is the manual gate in the release runbook,
    // so a green test here is not proof the feature works.
    assert!(
        plist.contains("<key>CFBundleURLTypes</key>"),
        "CFBundleURLTypes lost; tracecommons:// invite links would go dead:\n{plist}"
    );
    assert!(
        plist.contains("<string>tracecommons</string>"),
        "the tracecommons scheme is not declared:\n{plist}"
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
            .is_some_and(|flags| flags.contains("../target/debug"))
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

#[test]
fn release_tags_are_checked_against_the_versions_the_binaries_report() {
    let apps = read(".github/workflows/release-apps.yml");
    assert!(
        apps.contains("crates/trace-commons-contributor-gtk/Cargo.toml"),
        "the app release must compare its tag with the Linux shell package version"
    );
    assert!(
        apps.contains("PACKAGE_VERSION") && apps.contains("TAG_VERSION"),
        "the app release must refuse a tag/package version mismatch"
    );

    let contributor = read(".github/workflows/release-contributor.yml");
    for manifest in [
        "crates/trace-commons-contributor/Cargo.toml",
        "crates/trace-commons-contributor-ffi/Cargo.toml",
    ] {
        assert!(
            contributor.contains(manifest),
            "the contributor release must check {manifest} against its tag"
        );
    }
    assert!(
        contributor.contains("PACKAGE_VERSION") && contributor.contains("TAG_VERSION"),
        "the contributor release must refuse a tag/package version mismatch"
    );
    for required in [
        "HOMEBREW_TAP_TOKEN",
        "WINGET_PKGS_TOKEN",
        "TRACE_COMMONS_UPDATE_PUBLIC_KEY_HEX",
    ] {
        assert!(
            contributor.contains("missing configuration") && contributor.contains(required),
            "the contributor release must fail before building when {required} is absent"
        );
    }
    assert!(
        contributor.contains("needs: release-config"),
        "the signed contributor build must wait for release configuration preflight"
    );
}

/// The publish job's gate allows a partial run (at least one platform
/// succeeded), so the release notes must not unconditionally describe all
/// three platforms -- otherwise a Linux-only or macOS-only run tells
/// contributors on a platform that did NOT build to install from a URL
/// that either 404s or silently serves the previous release, and they end
/// up on an old build believing it is the new tag. Each platform's
/// paragraph, and the brew line specifically, must be conditioned on that
/// platform's job result.
#[test]
fn release_notes_only_describe_platforms_that_actually_succeeded() {
    let workflow = read(".github/workflows/release-apps.yml");
    for needed in [
        "needs.macos.result",
        "needs.windows.result",
        "needs.linux-flatpak.result",
    ] {
        assert!(
            workflow.contains(needed),
            "the publish step must branch the release notes on {needed}, \
             not assume every platform succeeded"
        );
    }
    // The brew line must sit INSIDE a shell conditional on MACOS_RESULT, not
    // merely after it. An earlier version of this test compared positions --
    // MACOS_RESULT appearing before `brew install --cask` -- which could not
    // fail: MACOS_RESULT is declared in the step's `env:` block, and a step's
    // env always precedes its `run:` body, so the assertion held even with the
    // brew line emitted unconditionally. Walk the run body instead and require
    // the line to be lexically inside a macOS test.
    let publish_step_start = workflow
        .find("name: Publish")
        .expect("publish step must exist");
    let publish_step = &workflow[publish_step_start..];
    let publish_lines: Vec<&str> = publish_step.lines().collect();
    let brew_line_idx = publish_lines
        .iter()
        .position(|line| line.contains("brew install --cask"))
        .expect("release notes must mention brew install");

    let mut guarded = false;
    for line in publish_lines[..brew_line_idx].iter().rev() {
        let t = line.trim();
        // A `fi` closes the nearest conditional before we reach an opening
        // test, so the brew line is outside it.
        if t == "fi" {
            break;
        }
        if t.starts_with("if ") && t.contains("MACOS_RESULT") {
            guarded = true;
            break;
        }
    }
    assert!(
        guarded,
        "the `brew install --cask` line must be emitted inside a shell \
         conditional on MACOS_RESULT. The cask ships only a macOS artifact, so \
         advertising it on a run whose macOS job failed points contributors at \
         something that does not exist."
    );
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

/// Both workflows sign Windows binaries with the same duplicated dlib logic,
/// so both must be pinned. Reading only one leaves the other free to drop the
/// timestamp -- and an untimestamped Trusted Signing signature keeps validating
/// for about three days, so no same-day run would catch it.
///
/// `release-apps.yml`'s windows job no longer inlines this logic: it calls
/// out to `scripts/windows/setup-trusted-signing.ps1` and
/// `scripts/windows/sign-with-trusted-signing.ps1`. `release-contributor.yml`
/// still inlines it. So for `release-apps.yml` the label under test is the
/// workflow text PLUS both extracted scripts' text -- the properties this
/// test pins are properties of what that job actually runs, wherever the
/// text for it now lives -- while `release-contributor.yml` is checked
/// against its own (still-inline) text alone.
#[test]
fn windows_signing_is_timestamped() {
    let apps = format!(
        "{}\n{}\n{}",
        read(".github/workflows/release-apps.yml"),
        read("scripts/windows/setup-trusted-signing.ps1"),
        read("scripts/windows/sign-with-trusted-signing.ps1"),
    );
    assert_windows_signing_is_hardened(
        ".github/workflows/release-apps.yml (+ scripts/windows/*.ps1)",
        &apps,
    );

    let contributor = read(".github/workflows/release-contributor.yml");
    assert_windows_signing_is_hardened(".github/workflows/release-contributor.yml", &contributor);
}

fn assert_windows_signing_is_hardened(label: &str, text: &str) {
    // Microsoft's dlib driven by signtool, NOT the marketplace action -- so the
    // client can be verified by content before it runs in a job that holds
    // signing authority.
    assert!(
        text.contains("Azure.CodeSigning.Dlib.dll"),
        "{label}: Windows signing drives Microsoft's Trusted Signing dlib via signtool"
    );
    assert!(
        !text.contains("azure/trusted-signing-action"),
        "{label}: the marketplace action was deliberately replaced by the SHA-verified \
         dlib; reintroducing it drops the content check"
    );
    assert!(
        text.contains("TRUSTED_SIGNING_CLIENT_SHA256")
            && text.contains("Refusing to expand a potentially tampered"),
        "{label}: the signing client must be verified by SHA-256 and fail closed before \
         extraction"
    );
    // Trusted Signing certificates are valid for roughly three days. Without
    // an RFC3161 countersignature the signature stops validating days after
    // release -- a failure no same-day test would catch.
    assert!(
        text.contains("/tr http://timestamp.acs.microsoft.com"),
        "{label}: every sign invocation needs an RFC3161 timestamp server: Trusted \
         Signing certificates carry ~3-day validity, so the countersignature \
         is the only reason a signature outlives them"
    );
    assert!(
        text.contains("/td SHA256"),
        "{label}: the timestamp digest algorithm must be pinned alongside /tr"
    );
    assert!(
        text.contains("signtool") || text.contains("Get-AuthenticodeSignature"),
        "{label}: the signature must be verified in the job, not assumed"
    );
    // Each real `signtool ... sign` invocation must carry its own /tr
    // timestamp flag. A `contains("/tr ")` check alone (as above) only
    // proves at least one sign call is timestamped -- it says nothing about
    // a second `signtool sign` added later without one, which would pass
    // that check while shipping an untimestamped binary. Comments mention
    // "/tr " in prose (see above), so count only non-comment lines.
    let executable_lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect();
    // Match the ACT of signing, not one spelling of it. An earlier version
    // filtered on the literal `TS_SIGNTOOL" sign`, so a sign call written with
    // any other variable or an absolute path counted as zero invocations and
    // made the >= comparison below vacuously true -- the exact regression this
    // assertion exists to catch.
    let sign_invocations = executable_lines
        .iter()
        .filter(|line| {
            let l = line.to_lowercase();
            // " sign " with BOTH spaces: `-Filter signtool.exe` contains
            // " signtool", whose prefix is " sign", so a looser test counts the
            // tool-discovery line as an invocation.
            l.contains("signtool") && l.contains(" sign ")
        })
        .count();
    assert!(
        sign_invocations > 0,
        "{label}: found no signtool sign invocation at all -- either the \
         Windows signing step was removed or this detector no longer matches it"
    );
    let tr_flags = executable_lines
        .iter()
        .filter(|line| line.contains("/tr "))
        .count();
    assert!(
        tr_flags >= sign_invocations,
        "{label}: found {sign_invocations} `signtool sign` invocation(s) but \
         only {tr_flags} /tr flag(s) -- every sign call must be timestamped"
    );
}

fn extract_between<'a>(haystack: &'a str, prefix: &str, suffix: char) -> &'a str {
    let start = haystack
        .find(prefix)
        .unwrap_or_else(|| panic!("expected to find {prefix:?}"))
        + prefix.len();
    let end = haystack[start..]
        .find(suffix)
        .unwrap_or_else(|| panic!("expected a closing {suffix:?} after {prefix:?}"));
    &haystack[start..start + end]
}

/// A comment in both workflows claims "the test pins the hash constant in
/// each", but the only check was that the identifier
/// TRUSTED_SIGNING_CLIENT_SHA256 appears in each file -- never that the two
/// files' hash and version constants actually AGREE. They are
/// byte-identical today, which makes the gap latent rather than visible: a
/// bump to one file without the other would sign with mismatched
/// expectations and nothing here would catch it.
///
/// The hash constant is still duplicated directly between the two workflow
/// files: `release-apps.yml`'s windows job passes its
/// `TRUSTED_SIGNING_CLIENT_SHA256` env var into
/// `scripts/windows/setup-trusted-signing.ps1` as `-ExpectedSha256` rather
/// than hardcoding it, but `release-contributor.yml` still hardcodes its own
/// copy inline -- so the two workflow-level values must still agree.
///
/// The nuget version pin moved when `release-apps.yml`'s windows job was
/// wired to the extracted scripts: it no longer sets `$nugetVersion` itself
/// and instead relies on `setup-trusted-signing.ps1`'s own default
/// (`-NuGetVersion`, unset by the caller). `release-contributor.yml` still
/// sets `$nugetVersion` inline. The duplication that must not drift is now
/// between the script's default and the contributor workflow's inline value.
#[test]
fn duplicated_windows_signing_constants_do_not_drift_between_workflows() {
    let apps = read(".github/workflows/release-apps.yml");
    let contributor = read(".github/workflows/release-contributor.yml");

    let apps_hash = extract_between(&apps, "TRUSTED_SIGNING_CLIENT_SHA256: ", '\n').trim();
    let contributor_hash =
        extract_between(&contributor, "TRUSTED_SIGNING_CLIENT_SHA256: ", '\n').trim();
    assert_eq!(
        apps_hash, contributor_hash,
        "release-apps.yml and release-contributor.yml pin different \
         TRUSTED_SIGNING_CLIENT_SHA256 values -- these must agree or one \
         workflow is trusting a different signing-client binary than the \
         other"
    );
    assert!(
        !apps.contains("$nugetVersion"),
        "release-apps.yml's windows job inlines $nugetVersion again -- it \
         should be relying on setup-trusted-signing.ps1's -NuGetVersion \
         default instead, or this test is checking the wrong value"
    );

    let script = read("scripts/windows/setup-trusted-signing.ps1");
    let apps_version = extract_between(&script, "NuGetVersion = '", '\'');
    let contributor_version = extract_between(&contributor, "$nugetVersion = \"", '"');
    assert_eq!(
        apps_version, contributor_version,
        "scripts/windows/setup-trusted-signing.ps1's default -NuGetVersion \
         and release-contributor.yml's inline $nugetVersion pin different \
         Microsoft.Trusted.Signing.Client nuget versions -- these must \
         agree, since the pinned SHA-256 is only valid for one version"
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

/// make-app-bundle.sh must build the FFI dylib for both Apple silicon and
/// Intel, lipo them into one dylib, and tell `swift build` to produce a
/// universal executable -- otherwise the app bundle silently only runs on
/// whichever architecture built it, which would still sign, notarize and
/// pass Gatekeeper before failing to launch for the other half of users.
/// Since the DMG this produces is now universal, its filename (read by the
/// cask-bump step's checksum lookup) must not claim a single architecture.
#[test]
fn macos_app_bundle_builds_and_lipos_both_architectures() {
    let script = read("macos/scripts/make-app-bundle.sh");
    assert!(
        script.contains("aarch64-apple-darwin") && script.contains("x86_64-apple-darwin"),
        "make-app-bundle.sh must build the FFI dylib for both \
         aarch64-apple-darwin and x86_64-apple-darwin"
    );
    assert!(
        script.contains("lipo -create"),
        "the two architecture-specific dylibs must be lipo'd into one \
         universal dylib before swift build links against them"
    );
    assert!(
        script.contains("--arch arm64") && script.contains("--arch x86_64"),
        "swift build must be passed --arch arm64 --arch x86_64 so the app \
         executable is universal, not just the dylib it links"
    );

    let workflow = read(".github/workflows/release-apps.yml");
    assert!(
        workflow.contains("TraceCommons-${SHORT_VERSION}.dmg") && !workflow.contains("-arm64.dmg"),
        "the DMG is universal now, so its filename must not carry an \
         architecture suffix"
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

/// summary.sig is not a bare detached OpenPGP signature -- it is an OSTree
/// GVariant whose `ostree.gpgsigs` key holds the detached signatures. A
/// `gpg --verify summary.sig summary` call always fails with "no valid
/// OpenPGP data found" against a real signed repo (confirmed empirically),
/// so that check would abort every publish regardless of whether the repo
/// was actually signed. The real check has to go through OSTree's own
/// summary-verification path (a remote with gpg-verify-summary plus a
/// pull), not a raw gpg invocation against the file.
#[test]
fn publish_script_does_not_gpg_verify_the_raw_summary_file() {
    let publish = read("scripts/flatpak/publish-repo.sh");
    let executable_lines: Vec<&str> = publish
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect();
    assert!(
        !executable_lines
            .iter()
            .any(|line| line.contains("gpg --verify")),
        "summary.sig is an OSTree GVariant, not a bare detached OpenPGP \
         signature -- `gpg --verify` against it always fails with \"no \
         valid OpenPGP data found\", even on a correctly signed repo, so \
         this check would abort every publication"
    );
    assert!(
        publish.contains("gpg-verify-summary"),
        "publish-repo.sh must verify the summary the way an OSTree/flatpak \
         client actually does: import the key into a remote with \
         gpg-verify-summary enabled and pull"
    );
}

/// `ostree show "$ref" | grep -qi signature` is a substring test, and
/// `ostree show`'s own no-signature and untrusted-key error paths BOTH
/// contain the word "signature" ("no signatures found", "Can't check
/// signature: public key not found") -- confirmed against a real unsigned
/// commit, which matches that grep despite carrying no signature at all.
#[test]
fn sign_script_checks_detached_metadata_not_a_signature_substring() {
    let script = read("scripts/flatpak/build-and-sign.sh");
    let executable_lines: Vec<&str> = script
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect();
    assert!(
        !executable_lines
            .iter()
            .any(|line| line.contains("show \"$ref\" | grep")),
        "a substring grep over `ostree show` output cannot distinguish \
         signed from unsigned: OSTree's own error messages for missing or \
         untrusted signatures both contain the word \"signature\""
    );
    assert!(
        script.contains("--print-detached-metadata-key=ostree.gpgsigs"),
        "the unsigned-repo guard must test for the detached ostree.gpgsigs \
         metadata key, which is absent (non-zero exit) on an unsigned \
         commit and present (zero exit) on a signed one"
    );
}

/// GitHub's workflow_dispatch ref selector accepts a tag ref, so
/// `startsWith(github.ref, 'refs/tags/')` alone is satisfied by dispatching
/// with ref `contributor-v0.1.0` -- a manual dispatch would then publish a
/// real release and open a tap PR, contradicting this file's own header
/// comment and docs/release-runbook.md, which both say a dispatch never
/// publishes. The gate must require the push event as well, exactly as
/// release-apps.yml's publish job does.
#[test]
fn contributor_publish_requires_a_tag_push_not_just_a_tag_ref() {
    let workflow = read(".github/workflows/release-contributor.yml");
    let publish_start = workflow
        .find("\n  publish:")
        .expect("publish job must exist");
    let publish_job = &workflow[publish_start..];
    let if_line_end = publish_job
        .find("runs-on:")
        .expect("publish job must have a runs-on");
    let gate = &publish_job[..if_line_end];
    assert!(
        gate.contains("github.event_name == 'push'"),
        "the publish job's `if:` must require github.event_name == 'push' \
         in addition to the tag-ref check, or a workflow_dispatch run with \
         a tag-shaped ref input can publish a real release"
    );
}

/// Return only the lines of a workflow that the runner actually executes,
/// so an assertion about what a step *does* cannot be satisfied (or
/// defeated) by a comment describing it.
fn executable_text(workflow: &str) -> String {
    workflow
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The tap bump must stay a pull request -- it is the audit trail, and the
/// place the tap's own checks run -- and it must merge itself. Eleven bump
/// pull requests accumulated unmerged back to 0.4.1 while the tap served
/// cask 0.4.0 and formula 0.3.0, because merging was a human step the
/// runbook never told anyone to take.
#[test]
fn tap_bumps_go_through_a_pull_request() {
    for file in [
        ".github/workflows/release-apps.yml",
        ".github/workflows/release-contributor.yml",
    ] {
        let workflow = read(file);
        assert!(
            workflow.contains("homebrew-tap"),
            "{file} must bump the tap"
        );
        // Still a pull request, not a direct push to the tap's default
        // branch: the pull request is where the tap's own checks run.
        assert!(
            workflow.contains("gh pr create"),
            "{file} must open a pull request against the tap, not push to it"
        );
    }
}

/// The bump pull request is merged by the workflow, not by a human. The
/// merge must use the non-bypassing form -- `--auto` waits for the tap's
/// required checks and branch protection -- and must never pass `--admin`,
/// which exists precisely to bypass them.
#[test]
fn tap_bumps_never_merge_themselves_ungated() {
    // Automating this merge is wanted, but TraceCommons/homebrew-tap has
    // `allow_auto_merge = false`, no branch protection on its default
    // branch, and no CI workflows at all. `gh pr merge --auto` would fail
    // outright there, and any fallback to a plain `--squash`/`--merge`
    // would be an immediate ungated merge into what `brew upgrade` serves
    // -- the direct push this step exists to avoid, wearing a pull request
    // as a disguise. The merge may be automated once the tap has an audit
    // check to gate on; until then this pins that it is not.
    for file in [
        ".github/workflows/release-apps.yml",
        ".github/workflows/release-contributor.yml",
    ] {
        let workflow = read(file);
        let exec = executable_text(&workflow);
        assert!(
            !exec.contains("gh pr merge"),
            "{file} must not merge the bump pull request: the tap has no \
             required check for an auto-merge to wait on, so any merge here \
             is ungated"
        );
        assert!(
            !exec.contains("--admin"),
            "{file} must never pass --admin to gh"
        );
        // The pointer is the whole mitigation while the merge is manual:
        // eleven bumps went unmerged back to 0.4.1 because nothing surfaced
        // them. It must reach the run summary, not just the log.
        assert!(
            workflow.contains("GITHUB_STEP_SUMMARY"),
            "{file} must surface the open bump on the run summary so it is \
             not forgotten the way 0.4.1 through 0.4.6 were"
        );
    }
}

/// A deleted-and-re-cut tag re-runs these steps against branches that
/// already exist. `git push -u origin "$BRANCH"` was rejected with
/// "! [rejected] (fetch first)" on the app-v0.4.7 re-run, failing the whole
/// publish job after every artifact had already published.
#[test]
fn bump_branch_pushes_survive_a_re_run() {
    for file in [
        ".github/workflows/release-apps.yml",
        ".github/workflows/release-contributor.yml",
    ] {
        let workflow = read(file);
        let pushes: Vec<&str> = workflow
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with("git push"))
            .collect();
        assert!(
            !pushes.is_empty(),
            "{file} must still push a bump branch somewhere"
        );
        for line in pushes {
            assert!(
                line.contains("--force"),
                "{file}: `{line}` is rejected on a re-run against an existing \
                 bump branch, which fails the release after its assets have \
                 already published"
            );
        }
        // Opening the pull request must tolerate one already existing for
        // that head: `gh pr create` exits non-zero on a duplicate head.
        assert!(
            workflow.contains(
                "gh pr list --repo TraceCommons/homebrew-tap --head \"$BRANCH\" \
                 --state open --json url"
            ) || workflow.contains("--head \"$BRANCH\" --state open --json url"),
            "{file} must reuse an already-open bump pull request instead of \
             erroring on a duplicate head"
        );
    }
}

/// The dangerous half of making the push forceful: a force-push carrying the
/// *previous* run's checksum would publish a cask whose sha256 does not match
/// the DMG, which reads to users as tampering. Verified by hand on
/// app-v0.4.7 -- the re-run's DMG hashed to d60ce434... while the branch left
/// behind by the first run still said 5234e018...
///
/// What makes that structurally impossible is that the bump branch is never
/// fetched. Each step clones `--single-branch` (the tap's, or the winget
/// fork's, default branch only) and rebuilds the branch from that plus a hash
/// computed from this run's own artifact, so there is nothing of a previous
/// attempt left to carry forward.
#[test]
fn force_pushed_bump_branches_cannot_carry_a_stale_checksum() {
    for (file, step) in [
        (
            ".github/workflows/release-apps.yml",
            "name: Open a cask bump",
        ),
        (
            ".github/workflows/release-contributor.yml",
            "name: Open a formula bump",
        ),
        (
            ".github/workflows/release-contributor.yml",
            "name: Generate and submit",
        ),
    ] {
        let workflow = read(file);
        let start = workflow
            .find(step)
            .unwrap_or_else(|| panic!("{file} must still contain `{step}`"));
        let body = &workflow[start..];
        let body = &body[..body.find("\n      - name:").unwrap_or(body.len())];
        let body = executable_text(body);
        assert!(
            body.contains("--single-branch"),
            "{file} / {step}: clone the default branch only. Fetching the \
             bump branch is the only route by which a previous run's \
             checksum could reach a force-push."
        );
        assert!(
            !body.contains("git fetch") && !body.contains("git pull"),
            "{file} / {step}: must not fetch the existing bump branch -- \
             re-deriving the branch from the default branch is what \
             guarantees the checksum came from this run"
        );
        // Re-running when the tap already carries this exact content must
        // not die on `git commit` finding nothing to commit.
        assert!(
            body.contains("git diff --quiet") || body.contains("git diff --cached --quiet"),
            "{file} / {step}: a re-run against an already-bumped tap must \
             exit cleanly rather than failing on an empty commit"
        );
    }
}

/// Every hash written into the tap must be re-derived from an artifact this
/// run produced, and proved to have landed in the file before anything is
/// pushed. These assertions are what stop a future edit from "helpfully"
/// reading a hash out of the existing bump branch.
#[test]
fn tap_bumps_prove_they_wrote_this_runs_checksum() {
    let apps = executable_text(&read(".github/workflows/release-apps.yml"));
    assert!(
        apps.contains("SHA=\"$(awk '{print $1}' dist/macos-dmg/TraceCommons-\"$V\".dmg.sha256)\""),
        "the cask bump must read the checksum from this run's own DMG artifact"
    );
    assert!(
        apps.contains("grep -qF \"$SHA\" Casks/trace-commons.rb"),
        "the cask bump must assert the computed checksum actually landed in \
         the cask before pushing, not merely that a substitution ran"
    );

    let contributor = executable_text(&read(".github/workflows/release-contributor.yml"));
    assert!(
        contributor.contains("ARM=\"$(awk '{print $1}' dist/aarch64-apple-darwin/")
            && contributor.contains("X86=\"$(awk '{print $1}' dist/x86_64-apple-darwin/"),
        "the formula bump must read both checksums from this run's own artifacts"
    );
    // The winget manifest hash is never passed in by hand: the generator
    // downloads the published asset and hashes the bytes it got.
    assert!(
        contributor.contains("./scripts/winget/generate-manifests.sh \"$V\""),
        "the winget manifest must be regenerated from the published asset on \
         every run rather than reusing a branch's InstallerSha256"
    );
}

/// `gh pr create --repo` selects the base repository but cannot infer a head
/// branch from a different workflow checkout. Both real 0.2.x tag runs pushed
/// their tap branches and then failed here with "use the --head flag". Pin the
/// explicit branch so a package-manager follow-up cannot fail the release job
/// after its assets have already been published.
#[test]
fn cross_repository_pull_requests_name_their_head_branches() {
    let apps = read(".github/workflows/release-apps.yml");
    assert!(
        apps.contains("gh pr create --fill --repo TraceCommons/homebrew-tap --head \"$BRANCH\""),
        "the app cask PR must explicitly name its pushed head branch"
    );

    let contributor = read(".github/workflows/release-contributor.yml");
    assert!(
        contributor
            .contains("gh pr create --fill --repo TraceCommons/homebrew-tap --head \"$BRANCH\""),
        "the contributor formula PR must explicitly name its pushed head branch"
    );
    assert!(
        contributor.contains("FORK_OWNER=\"$(gh api user --jq .login)\""),
        "the winget job must derive the owner of the token-scoped fork"
    );
    // The winget pull request is deliberately NOT opened by the workflow:
    // no token we can issue is permitted to call `createPullRequest`
    // against microsoft/winget-pkgs, and attempting it failed the whole
    // release on contributor-v0.4.7 after every artifact had published.
    // What must survive is the pointer to the branch the job pushed, since
    // that is now the only route to the manifest reaching winget users.
    assert!(
        !contributor.contains("gh pr create --fill --repo microsoft/winget-pkgs"),
        "the winget job must not try to open a pull request it has no token for"
    );
    assert!(
        contributor.contains("${FORK_OWNER}:${BRANCH}"),
        "the winget job must print a compare URL naming the fork-owned branch"
    );
    assert!(
        contributor.contains("GITHUB_STEP_SUMMARY"),
        "the winget compare URL must reach the run summary, not just the log"
    );
}

/// The staleness backstop must exist, must be scheduled, and must stay out
/// of anything that gates a release or a code pull request -- a previous
/// release's unmerged bump failing *this* release's publish job is the
/// failure shape the rest of this work removed.
#[test]
fn a_scheduled_job_watches_for_unmerged_tap_bumps() {
    let staleness = read(".github/workflows/tap-bump-staleness.yml");
    assert!(
        staleness.contains("schedule:") && staleness.contains("cron:"),
        "the staleness check must run on a schedule, not only on demand"
    );
    let triggers = staleness
        .split("\njobs:")
        .next()
        .expect("the staleness workflow must have a jobs: block");
    assert!(
        !triggers.contains("\n  pull_request:") && !triggers.contains("\n  push:"),
        "the staleness check must not run on pushes or pull requests: it \
         would be constant noise for a condition that changes once per \
         release, and a fork pull request cannot see HOMEBREW_TAP_TOKEN"
    );
    assert!(
        staleness.contains("TraceCommons/homebrew-tap"),
        "the staleness check must actually query the tap"
    );
    assert!(
        staleness.contains("24 hours ago"),
        "a bump opened by a release still in flight is not stale; the check \
         needs an age floor"
    );

    for file in [
        ".github/workflows/release-apps.yml",
        ".github/workflows/release-contributor.yml",
        ".github/workflows/ci.yml",
    ] {
        // Checked against executable lines only: a comment pointing at the
        // backstop is useful documentation and cannot create a dependency.
        // What must not exist is a `needs:`, a `uses:`, or a call.
        assert!(
            !executable_text(&read(file)).contains("tap-bump-staleness"),
            "{file} must not depend on the staleness check -- it must never \
             be able to fail a release or a code pull request over a \
             previous release's leftovers"
        );
    }
}

/// The runbook is the other half of the fix. It previously stopped at
/// "opens a pull request" and never told anyone to merge it, which is the
/// root cause of the nine stale pull requests.
#[test]
fn runbook_tells_the_releaser_to_merge_the_tap_bump() {
    // Task 12 previously stopped at "opens a pull request" and never told
    // anyone to merge it. That omission is why cask 0.4.1 through 0.4.6 sat
    // unmerged while Homebrew served 0.4.0.
    let runbook = read("docs/release-runbook.md");
    assert!(
        runbook.contains("merge"),
        "the runbook must tell the releaser to merge the bump pull request"
    );
    assert!(
        runbook.contains("tap-bump-staleness"),
        "the runbook must name the backstop that catches a bump nobody merged"
    );
}

#[test]
fn runbook_states_why_zap_spares_the_device_key() {
    let runbook = read("docs/release-runbook.md");
    assert!(
        runbook.contains("contributor.json"),
        "the runbook must name the file the cask's zap stanza spares"
    );
    assert!(
        runbook.contains("not idempotent"),
        "the runbook must say WHY: /v1/onboard is not idempotent, so deleting \
         the device key burns an invite code that cannot be reissued"
    );
}

/// macos-13 is a retired GitHub-hosted runner image. A job targeting a
/// retired image label does not fail -- it queues forever, confirmed
/// empirically against this exact matrix entry. x86_64-apple-darwin must be
/// built as a cross build on macos-14 instead.
#[test]
fn no_workflow_references_the_retired_macos_13_runner() {
    for path in [
        ".github/workflows/release-apps.yml",
        ".github/workflows/release-contributor.yml",
    ] {
        let workflow = read(path);
        let executable_lines: Vec<&str> = workflow
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .collect();
        assert!(
            !executable_lines
                .iter()
                .any(|line| line.contains("macos-13")),
            "{path}: macos-13 is a retired runner image -- a job targeting \
             it queues forever instead of failing, silently starving that \
             matrix leg. Build x86_64-apple-darwin as a cross build on \
             macos-14 instead."
        );
    }
    let contributor = read(".github/workflows/release-contributor.yml");
    assert!(
        contributor.contains("x86_64-apple-darwin"),
        "the Intel macOS CLI target must still be built somewhere"
    );
}

/// The formula-bump awk substitutes the two `sha256 "..."` lines by
/// position, which silently swaps hashes onto the wrong architecture if the
/// formula's blocks were ever reordered, and silently does nothing at all
/// if the formula used the single-line `sha256 arm: "...", x86_64: "..."`
/// form (the regex would never match, yet `git commit -am` still succeeds
/// because the version sed alone changed a line). Both failure shapes must
/// be caught: an explicit ordering assertion, and a post-substitution check
/// that both computed hashes actually landed in the file.
#[test]
fn formula_bump_verifies_its_own_substitution() {
    let workflow = read(".github/workflows/release-contributor.yml");
    let bump_start = workflow
        .find("name: Open a formula bump")
        .expect("formula bump step must exist");
    let bump_step = &workflow[bump_start..];
    assert!(
        bump_step.contains("a < i") || bump_step.contains("a<i"),
        "the bump step must assert on_arm precedes on_intel before relying \
         on positional hash substitution"
    );
    assert!(
        bump_step.contains("grep -qF \"$ARM\"") && bump_step.contains("grep -qF \"$X86\""),
        "the bump step must assert both computed hashes actually landed in \
         the formula after substitution, or a non-matching regex form \
         silently bumps the version while keeping the old checksums"
    );
}

/// release-apps.yml deliberately keeps contents: write off the
/// workflow-level permissions block and grants it only to the publish job.
/// release-contributor.yml's build job imports a .p12, holds the ASC notary
/// key and carries the Trusted Signing OIDC session -- the same signing
/// authority -- so it must make the same call, not the opposite one.
#[test]
fn contributor_workflow_scopes_contents_write_to_publish_only() {
    let workflow = read(".github/workflows/release-contributor.yml");
    let jobs_start = workflow.find("\njobs:").expect("jobs: block must exist");
    let (top_level, jobs) = workflow.split_at(jobs_start);
    assert!(
        top_level.contains("contents: read"),
        "workflow-level permissions must stay contents: read, matching \
         release-apps.yml -- the build job holds signing authority and \
         must not also inherit repo-write"
    );
    let publish_start = jobs.find("\n  publish:").expect("publish job must exist");
    let publish_job = &jobs[publish_start..];
    assert!(
        publish_job.contains("contents: write"),
        "the publish job must grant itself contents: write at the job \
         level, since the workflow level no longer does"
    );
}

/// The first real release (app-v0.1.0, run 31959830328) failed the flatpak
/// build because org.freedesktop.Sdk.Extension.rust-stable ships a rustc far
/// short of this crate's rust-version floor, and the build is
/// network-sandboxed so rustup cannot rescue it. The fix was to bundle a
/// pinned rust toolchain as a manifest source instead of depending on the
/// SDK extension's version at all.
#[test]
fn flatpak_manifest_bundles_a_pinned_rust_toolchain_per_arch() {
    let manifest =
        read("crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.yml");
    for (arch, url, sha256) in [
        (
            "x86_64",
            "https://static.rust-lang.org/dist/rust-1.92.0-x86_64-unknown-linux-gnu.tar.xz",
            "d2ccef59dd9f7439f2c694948069f789a044dc1addcc0803613232af8f88ee0c",
        ),
        (
            "aarch64",
            "https://static.rust-lang.org/dist/rust-1.92.0-aarch64-unknown-linux-gnu.tar.xz",
            "3e383f8b4fca710d0600d0c1de97b78281672be2cda6575ecbe1c183a12e3822",
        ),
    ] {
        assert!(
            manifest.contains(url),
            "manifest must pin a rust 1.92.0 tarball for {arch}: {url}"
        );
        assert!(
            manifest.contains(sha256),
            "manifest must pin the sha256 for the {arch} rust tarball"
        );
        // Each tarball source must be scoped to its own arch, or an x86_64
        // build would also fetch the aarch64 tarball (and vice versa).
        let url_pos = manifest
            .find(url)
            .unwrap_or_else(|| panic!("{arch} url must be present"));
        let source_start = manifest[..url_pos]
            .rfind("- type: archive")
            .unwrap_or_else(|| {
                panic!("{arch} tarball must be declared as a `type: archive` source")
            });
        let source = &manifest[source_start..url_pos];
        assert!(
            source.contains(&format!("only-arches: [{arch}]")),
            "the {arch} rust tarball source must be scoped with \
             only-arches: [{arch}], or the other arch's build would fetch \
             it too"
        );
    }
}

#[test]
fn flatpak_manifest_no_longer_references_the_rust_stable_sdk_extension() {
    let manifest =
        read("crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.yml");
    assert!(
        !manifest.contains("org.freedesktop.Sdk.Extension.rust-stable"),
        "the manifest must not reference the rust-stable SDK extension \
         anymore -- the build now uses its own bundled, pinned toolchain, \
         which was the whole point of bundling it"
    );
    assert!(
        !manifest.contains("/usr/lib/sdk/rust-stable/bin"),
        "the manifest must not append the SDK extension's rust bin dir to \
         PATH anymore; the bundled toolchain's own bin dir replaces it"
    );
}

#[test]
fn flatpak_manifest_pinned_toolchain_meets_the_crates_rust_version_floor() {
    let manifest =
        read("crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.yml");
    let cargo_toml = read("crates/trace-commons-contributor-gtk/Cargo.toml");
    let required = cargo_toml
        .lines()
        .find_map(|l| l.trim().strip_prefix("rust-version = \""))
        .and_then(|s| s.strip_suffix('"'))
        .expect("crates/trace-commons-contributor-gtk/Cargo.toml must declare rust-version");
    let pinned = manifest
        .lines()
        .find_map(|l| {
            let l = l.trim();
            l.strip_prefix("url: https://static.rust-lang.org/dist/rust-")
        })
        .and_then(|rest| rest.split('-').next())
        .expect("manifest must pin a rust toolchain url of the form rust-<version>-<arch>-...");
    let mut required_parts = required.split('.').map(|p| p.parse::<u32>().unwrap());
    let mut pinned_parts = pinned.split('.').map(|p| p.parse::<u32>().unwrap());
    let required_tuple = (
        required_parts.next().unwrap_or(0),
        required_parts.next().unwrap_or(0),
        required_parts.next().unwrap_or(0),
    );
    let pinned_tuple = (
        pinned_parts.next().unwrap_or(0),
        pinned_parts.next().unwrap_or(0),
        pinned_parts.next().unwrap_or(0),
    );
    assert!(
        pinned_tuple >= required_tuple,
        "the manifest pins rust {pinned} but crates/trace-commons-contributor-gtk/Cargo.toml \
         requires rust-version = \"{required}\"; bump the pinned tarball \
         (url + sha256, both arches) before releasing"
    );
}

/// Runs info-plist.sh with a given TC_SPARKLE_PUBLIC_ED_KEY (None = unset).
///
/// Unix-only for the same reason as its two callers below: it shells out to
/// bash. It carries its own `cfg` rather than relying on theirs, because an
/// ungated helper whose only callers are gated is dead code on Windows, and
/// this crate builds its tests under `-D warnings`.
#[cfg(unix)]
fn info_plist_with_key(key: Option<&str>) -> String {
    let script = repo_root().join("macos/scripts/info-plist.sh");
    let mut command = Command::new("bash");
    command.arg(&script).args(["0.4.2", "17"]);
    match key {
        Some(value) => {
            command.env("TC_SPARKLE_PUBLIC_ED_KEY", value);
        }
        None => {
            command.env_remove("TC_SPARKLE_PUBLIC_ED_KEY");
        }
    }
    let output = command.output().expect("failed to run info-plist.sh");
    assert!(
        output.status.success(),
        "info-plist.sh failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Unix-only: runs `macos/scripts/info-plist.sh` through bash.
#[cfg(unix)]
#[test]
fn info_plist_carries_the_approved_sparkle_configuration() {
    let plist = info_plist_with_key(Some("dGVzdC1wdWJsaWMta2V5LWJhc2U2NC12YWx1ZQ=="));

    assert!(
        plist.contains(
            "<key>SUFeedURL</key><string>\
             https://storage.googleapis.com/tracecommons-flatpak/updates/appcast.xml</string>"
        ),
        "the appcast feed URL is wrong or missing:\n{plist}"
    );
    assert!(
        plist.contains(
            "<key>SUPublicEDKey</key><string>dGVzdC1wdWJsaWMta2V5LWJhc2U2NC12YWx1ZQ==</string>"
        ),
        "the EdDSA public key was not injected:\n{plist}"
    );
    // Checks on, install off. Sparkle checks in the background without ever
    // asking permission to check, and nothing is replaced until a person
    // says yes. Flipping SUAutomaticallyUpdate to true would make this app
    // swap its own bytes silently, which the design forbids.
    assert!(
        plist.contains("<key>SUEnableAutomaticChecks</key><true/>"),
        "automatic checks are not enabled:\n{plist}"
    );
    assert!(
        plist.contains("<key>SUAutomaticallyUpdate</key><false/>"),
        "automatic install must stay off:\n{plist}"
    );
    assert!(
        plist.contains("<key>SUScheduledCheckInterval</key><integer>86400</integer>"),
        "the daily check interval is wrong or missing:\n{plist}"
    );
}

/// Unix-only: runs `macos/scripts/info-plist.sh` through bash.
#[cfg(unix)]
#[test]
fn info_plist_ships_no_feed_at_all_without_a_public_key() {
    // Fail closed. A bundle with a feed but no key would ask Sparkle to
    // fetch an appcast it cannot authenticate; a bundle with neither simply
    // has no update path, which is the correct state for a dev build.
    let plist = info_plist_with_key(None);
    assert!(
        !plist.contains("SUFeedURL"),
        "a keyless bundle must not carry a feed URL:\n{plist}"
    );
    assert!(
        !plist.contains("SUPublicEDKey"),
        "a keyless bundle must not carry an empty key:\n{plist}"
    );
    assert!(
        plist.contains("<key>SUEnableAutomaticChecks</key><false/>"),
        "a keyless bundle must not enable automatic checks:\n{plist}"
    );
}

#[test]
fn the_release_script_refuses_without_the_sparkle_public_key() {
    let script = read("macos/scripts/make-release-dmg.sh");
    assert!(
        script.contains("TC_SPARKLE_PUBLIC_ED_KEY"),
        "make-release-dmg.sh must require TC_SPARKLE_PUBLIC_ED_KEY. A release \
         built without it ships an app that can never receive an update, and \
         nothing about the DMG would look wrong."
    );
}

#[test]
fn the_release_workflow_passes_the_sparkle_public_key() {
    let workflow = read(".github/workflows/release-apps.yml");
    assert!(
        workflow.contains("TC_SPARKLE_PUBLIC_ED_KEY: ${{ secrets.SPARKLE_PUBLIC_ED_KEY }}"),
        "the macOS release job must pass the Sparkle public key through"
    );
}

#[test]
fn the_bundle_script_embeds_sparkle_with_ditto() {
    let script = read("macos/scripts/make-app-bundle.sh");
    assert!(
        script.contains("Sparkle.xcframework"),
        "make-app-bundle.sh must locate the Sparkle XCFramework. SwiftPM \
         links it but never embeds it; without a copy step the signed, \
         notarized app crashes on launch with 'Library not loaded'."
    );
    assert!(
        script.contains("macos-arm64_x86_64"),
        "the universal XCFramework slice must be named explicitly"
    );
    assert!(
        script.contains("ditto"),
        "a framework must be copied with ditto: Versions/Current, Resources \
         and the top-level binary are symlinks, and a copy that dereferences \
         them produces a bundle codesign rejects"
    );
}

#[test]
fn no_script_signs_sparkle_with_deep() {
    for path in [
        "macos/scripts/make-app-bundle.sh",
        "macos/scripts/make-release-dmg.sh",
    ] {
        let script = read(path);
        for line in script.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with('#') {
                continue;
            }
            if trimmed.contains("codesign") && trimmed.contains("--sign") {
                assert!(
                    !line.contains("--deep"),
                    "{path}: codesign --sign --deep re-signs Sparkle's Downloader \
                     XPC service without its entitlements. Sign inside-out \
                     instead. Offending line: {line}"
                );
            }
        }
    }
}

#[test]
fn the_bundle_script_signs_sparkle_inside_out() {
    let script = read("macos/scripts/make-app-bundle.sh");
    let order = [
        "XPCServices/Installer.xpc",
        "XPCServices/Downloader.xpc",
        "Versions/B/Autoupdate",
        "Versions/B/Updater.app",
    ];
    let mut previous = 0usize;
    for needle in order {
        let at = script
            .find(needle)
            .unwrap_or_else(|| panic!("make-app-bundle.sh never mentions {needle}"));
        assert!(
            at > previous,
            "{needle} is signed out of order; nested code must be signed \
             before the framework that seals it"
        );
        previous = at;
    }
    assert!(
        script.contains("--preserve-metadata=entitlements"),
        "Downloader.xpc must be signed with --preserve-metadata=entitlements \
         (Sparkle >= 2.6), or it loses the entitlement it needs"
    );
}

#[test]
fn the_release_script_signs_every_sparkle_component_for_notarization() {
    let script = read("macos/scripts/make-release-dmg.sh");
    for needle in [
        "XPCServices/Installer.xpc",
        "XPCServices/Downloader.xpc",
        "Versions/B/Autoupdate",
        "Versions/B/Updater.app",
        "--preserve-metadata=entitlements",
    ] {
        assert!(
            script.contains(needle),
            "make-release-dmg.sh must sign {needle}. Notarization rejects the \
             whole submission when any nested Mach-O lacks a Developer ID \
             signature, a secure timestamp, or the hardened runtime."
        );
    }
    // Everything nested must be signed before the app bundle that seals it.
    let last_sparkle = script
        .rfind("Sparkle.framework")
        .expect("make-release-dmg.sh never mentions Sparkle.framework");
    let outer_sign = script
        .find("--sign \"$MACOS_SIGNING_IDENTITY\" \"$APP\"")
        .expect("the outer app signing call changed shape");
    assert!(
        last_sparkle < outer_sign,
        "Sparkle is signed after the app bundle; signing the outer bundle \
         first is invalidated the moment anything inside it is touched"
    );
}

#[test]
fn the_release_workflow_publishes_the_appcast() {
    let wf = read(".github/workflows/release-apps.yml");
    assert!(
        wf.contains("generate-appcast.sh"),
        "release-apps.yml must run generate-appcast.sh. Without it SUFeedURL \
         404s and every Sparkle check fails closed and silently -- the app \
         reports no update forever and nothing logs an error."
    );
    assert!(
        wf.contains("sparkle-signing-key"),
        "the appcast must be signed with the Sparkle EdDSA key from Secret Manager"
    );
    assert!(
        wf.contains("appcast.xml"),
        "the generated appcast must be uploaded to the bucket"
    );
}

/// Extracts the text of one top-level job block from a workflow file: from
/// `\n  <name>:` up to (but not including) the next line matching `\n  <ident>:`
/// at the same two-space indent, or end of file.
fn extract_job<'a>(workflow: &'a str, name: &str) -> &'a str {
    let marker = format!("\n  {name}:");
    let start = workflow
        .find(&marker)
        .unwrap_or_else(|| panic!("expected to find job {name}"));
    let body_start = start + marker.len();
    let rest = &workflow[body_start..];
    // `split_inclusive` keeps the terminator in each item, so `line.len()` is
    // the true byte width whether the file ends its lines with LF or CRLF.
    // `str::lines()` strips a trailing `\r` but the cursor here only added one
    // byte back, so under CRLF every line under-counted by one, the drift
    // accumulated down the file, and the job body was returned truncated --
    // which surfaced as an assertion claiming a step near the end of the job
    // did not exist. See `workflow_parsing_survives_a_crlf_checkout`.
    let end = rest
        .split_inclusive('\n')
        .scan(0usize, |pos, line| {
            let this_pos = *pos;
            *pos += line.len();
            Some((this_pos, line))
        })
        .find(|(_, line)| {
            let line = line.trim_end_matches(['\r', '\n']);
            !line.is_empty()
                && line.starts_with("  ")
                && !line.starts_with("    ")
                && line.trim_end().ends_with(':')
        })
        .map(|(pos, _)| pos)
        .unwrap_or(rest.len());
    &workflow[start..body_start + end]
}

/// The MSIX build's own layout puts the package one directory deeper and
/// under the MSBuild project's name, not the packaging identity -- so the
/// filename discovered on disk is NOT assumed to be
/// `Iqlusion.TraceCommons_<quad>_x64.msix`. If the job published under
/// a name it invented (or a hardcoded one) while the feed's MainPackage@Uri
/// named something else, every client would resolve the feed successfully
/// and then 404 fetching the package -- a silent no-op with no error surface
/// anywhere. The fix is architectural: exactly one source of truth for the
/// filename (`steps.pkg.outputs.name`, itself discovered via a glob rather
/// than assumed), threaded unchanged into the feed generator's
/// `-PackageFileName` and the `gcloud storage cp` destination object name.
/// This test pins that architecture in text so a future edit that
/// reintroduces a second, divergent name is caught here instead of first
/// being noticed by a contributor's update silently doing nothing.
#[test]
fn windows_msix_publish_name_matches_appinstaller_feed_name() {
    let workflow = read(".github/workflows/release-apps.yml");
    let job = extract_job(&workflow, "windows-app");

    // The package name is discovered from disk, never spelled out as a
    // literal `Iqlusion.TraceCommons_...msix` anywhere in the job.
    assert!(
        job.contains("Get-ChildItem -Recurse -Filter *.msix -Path windows\\dist\\msix"),
        "the job must discover the built package's name by globbing the \
         real output directory rather than assuming a filename"
    );
    assert!(
        !job.contains("Iqlusion.TraceCommons_"),
        "the job must never hardcode a package filename derived from the \
         packaging identity -- MSBuild's Test-layout output is named after \
         the project, not the identity, and the two can diverge"
    );

    // steps.pkg.outputs.name is the single source of truth: it must appear
    // in a `PKG_NAME` env binding at least twice -- once for the step that
    // builds the feed (which sets -PackageFileName from it) and once for the
    // step that uploads the package to the bucket (which uses it as the
    // destination object name). If either step invented its own name, this
    // count would drop to a value that no longer entails they're the same
    // string.
    let pkg_name_bindings = job
        .matches("PKG_NAME: ${{ steps.pkg.outputs.name }}")
        .count();
    assert!(
        pkg_name_bindings >= 2,
        "expected at least 2 steps to bind PKG_NAME from steps.pkg.outputs.name \
         (found {pkg_name_bindings}) -- the feed-generation step and the \
         publish step must both derive the object name from the same \
         discovered value, or the feed's Uri and the uploaded object name \
         can silently diverge"
    );

    // The feed generator must be invoked with -PackageFileName bound to
    // that same env var, not a literal or a differently-derived value.
    let feed_step_start = job
        .find("Generate the appinstaller feed")
        .expect("the feed-generation step must exist");
    let feed_step = &job[feed_step_start..];
    let make_appinstaller_end = feed_step
        .find("make-appinstaller.ps1")
        .expect("the feed step must call make-appinstaller.ps1")
        + "make-appinstaller.ps1".len();
    let call_block_end = feed_step[make_appinstaller_end..]
        .find("-OutputPath")
        .expect("the make-appinstaller.ps1 call must set -OutputPath")
        + make_appinstaller_end
        + "-OutputPath".len();
    let call_block = &feed_step[..call_block_end];
    assert!(
        call_block.contains("-PackageFileName $env:PKG_NAME"),
        "make-appinstaller.ps1 must be called with -PackageFileName bound to \
         $env:PKG_NAME (steps.pkg.outputs.name), not a literal filename"
    );

    // The publish step must upload under that same PKG_NAME, to
    // windows/$PKG_NAME in the bucket -- exactly what the feed just named.
    let publish_step_start = job
        .rfind("name: Publish")
        .expect("the publish step must exist");
    let publish_step = &job[publish_step_start..];
    assert!(
        publish_step.contains("gs://$env:BUCKET/windows/$env:PKG_NAME"),
        "the publish step must upload the package to windows/$PKG_NAME in \
         the bucket -- the exact filename the feed's MainPackage@Uri names. \
         Uploading under any other name leaves the feed pointing at an \
         object that does not exist."
    );

    // And the package must go up before the feed: a feed naming an object
    // not yet present is a window where every update check 404s.
    let pkg_upload_pos = publish_step
        .find("gs://$env:BUCKET/windows/$env:PKG_NAME")
        .unwrap();
    let feed_upload_pos = publish_step
        .find("gs://$env:BUCKET/windows/TraceCommons.appinstaller")
        .expect("the publish step must also upload the feed");
    assert!(
        pkg_upload_pos < feed_upload_pos,
        "the package must be uploaded before the feed, so there is never a \
         window in which the feed names an object that is not there yet"
    );
}

#[test]
fn windows_msix_job_installs_and_verifies_before_publishing() {
    let workflow = read(".github/workflows/release-apps.yml");
    let job = extract_job(&workflow, "windows-app");

    assert!(job.contains("environment: release"));
    for needle in [
        "Add-AppxPackage -Path $env:PKG",
        "the package did not register",
        "does not match the stamped",
        "Confirm the state directory is not virtualized",
        "Invoke-CommandInDesktopPackage",
        "Write virtualization is on",
        // The probe must be able to name its cause. A missing file at the
        // real path is equally consistent with virtualization being on and
        // with the container process never having started, so the step also
        // watches the redirected location and reads back a transcript the
        // container process writes outside the redirected tree.
        "LocalCache\\Local\\trace-commons",
        "container process transcript",
        // Get-AppxPackage returning nothing or more than one entry leaves
        // PackageFamilyName empty or an array, and
        // Invoke-CommandInDesktopPackage then silently does nothing.
        "expected exactly 1 registered",
    ] {
        assert!(
            job.contains(needle),
            "windows-app job is missing expected content: {needle}"
        );
    }

    // Publication must only happen on a push (tag release), never on a
    // workflow_dispatch -- but the build/sign/install/verify steps above
    // must run unconditionally so the whole path is provable without ever
    // moving what an installed client actually pulls.
    let publish_gcp_auth = job
        .find("Authenticate to GCP")
        .expect("the publish path must authenticate to GCP");
    let auth_step = &job[publish_gcp_auth..];
    let if_line = auth_step
        .lines()
        .find(|l| l.trim_start().starts_with("if:"))
        .expect("the GCP auth step must be gated");
    assert!(
        if_line.contains("github.event_name == 'push'"),
        "publication must be gated to push events only, found: {if_line}"
    );
}

#[test]
fn windows_msix_job_is_wired_into_the_release_job_gate_and_artifacts() {
    let workflow = read(".github/workflows/release-apps.yml");
    assert!(
        workflow.contains("\n  windows-app:"),
        "release-apps.yml must define the windows-app job"
    );

    let publish_job = extract_job(&workflow, "publish");

    assert!(
        publish_job.contains("needs: [version, macos, windows, windows-app, linux-flatpak]"),
        "the publish job must depend on windows-app"
    );
    assert!(
        publish_job.contains("needs.windows-app.result == 'success'"),
        "the publish job's gate must treat windows-app as a platform whose \
         success alone justifies running publish"
    );
    assert!(
        publish_job.contains("windows-msix"),
        "the publish job must download the windows-msix artifact"
    );
    assert!(
        publish_job.contains("WINDOWS_APP_RESULT"),
        "the publish job's release-notes step must branch on WINDOWS_APP_RESULT"
    );
    assert!(
        publish_job.contains("TraceCommons.appinstaller"),
        "the release notes must point contributors at the .appinstaller feed"
    );
}

#[test]
fn ci_packages_and_validates_the_windows_app_feed_identity() {
    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.contains("-p:TcPackaged=true"),
        "CI must opt into the packaged WinUI flavour; setting only \
         GenerateAppxPackageOnBuild cannot override WindowsPackageType=None"
    );
    assert!(
        ci.contains("-p:GenerateAppxPackageOnBuild=true"),
        "CI must build a real package (not just compile) so \
         Package.appxmanifest, MakePri and the visual assets are exercised \
         on every pull request"
    );
    assert!(
        ci.contains("Confirm exactly one package was produced"),
        "CI must assert exactly one .msix was produced, the same shape as \
         the release job's own check"
    );
    assert!(
        ci.contains("make-appinstaller.ps1"),
        "CI must exercise the feed generator against the real manifest \
         identity so a rename on either side fails on every PR, not just \
         at release time"
    );
    assert!(
        ci.contains("feed package name $($main.Name) does not match manifest identity"),
        "CI must assert the feed's MainPackage name matches \
         Package.appxmanifest's Identity/@Name"
    );

    let project = read("windows/src/TraceCommons.App/TraceCommons.App.csproj");
    assert_eq!(
        project.matches("<AppxManifest Include=").count(),
        1,
        "the WinUI project must register exactly one package manifest"
    );
    assert!(
        project.contains("../../packaging/Package.appxmanifest"),
        "the WinUI project must use the canonical packaging manifest"
    );
    assert!(
        project.contains("<TargetPlatformMinVersion>10.0.17763.0</TargetPlatformMinVersion>"),
        "MSIX's 19041 floor must not narrow the default unpackaged build's support"
    );

    let manifest = read("windows/packaging/Package.appxmanifest");
    assert!(
        manifest.contains("Name=\"Iqlusion.TraceCommons\""),
        "the canonical manifest must retain the established package identity"
    );
    assert!(
        manifest.contains("<rescap:Capability Name=\"packageManagement\" />"),
        "the packaged app needs packageManagement to apply updates on demand"
    );

    let release = read(".github/workflows/release-apps.yml");
    let job = extract_job(&release, "windows-app");
    for expected in [
        "-ManifestPath windows\\packaging\\Package.appxmanifest",
        "-p:TcPackaged=true",
        "Get-AppxPackage -Name Iqlusion.TraceCommons",
        "-AppId App",
        "-PackageName Iqlusion.TraceCommons",
    ] {
        assert!(
            job.contains(expected),
            "the Windows release job must consistently use the canonical MSIX identity: {expected}"
        );
    }
}

/// A Windows checkout has CRLF line endings -- `core.autocrlf` defaults to
/// true there, and the box this was found on reported 1367 CRLF pairs in
/// `release-apps.yml` alone. That must not change what these tests read.
///
/// The bug this pins was silent and total. `extract_job` walked lines with
/// `str::lines()`, which strips a trailing `\r`, while advancing its byte
/// cursor by `line.len() + 1`. Under CRLF every line under-counted by one
/// byte, the drift accumulated down the file, and the job body was sliced
/// short -- so an assertion about a step near the END of a job failed
/// claiming the step was absent, which is indistinguishable from the step
/// genuinely having been deleted. It cost a Windows debugging session to
/// tell those two apart.
#[test]
fn workflow_parsing_survives_a_crlf_checkout() {
    let lf = read(".github/workflows/release-apps.yml");
    let crlf = lf.replace('\n', "\r\n");

    for job in ["windows-app", "macos", "version"] {
        let from_lf = extract_job(&lf, job);
        let from_crlf = extract_job(&crlf, job).replace("\r\n", "\n");
        assert_eq!(
            from_lf, from_crlf,
            "extract_job({job}) disagreed between an LF and a CRLF checkout"
        );
    }
}

/// The flatpak's vendored source set must name exactly the crates the GTK
/// lockfile does.
///
/// The flatpak build runs `cargo --offline build` against `cargo-sources.json`,
/// so a crate in the lockfile with no vendor entry fails there and only there.
/// That is a bad place to find out: the GTK crate is a separate workspace
/// excluded from the root one, nothing in normal CI builds it, and the failure
/// surfaces on an `app-v*` tag after the release has already started.
///
/// It has now happened twice for the same reason. A dependency lands in a crate
/// the shell depends on -- `near-ai-privacy-filter` picking up `futures`, then
/// `tiktoken-rs` -- the lockfile is regenerated, and the vendor set is not.
/// Regenerating one without the other is invisible until the tag.
///
/// Checked in both directions. A missing entry breaks the build; a stale one is
/// harmless but means the set was hand-edited against a lockfile that has since
/// moved, which is the state this test exists to catch early.
#[test]
fn the_flatpak_vendor_set_matches_the_gtk_lockfile() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let lock =
        std::fs::read_to_string(root.join("crates/trace-commons-contributor-gtk/Cargo.lock"))
            .expect("GTK Cargo.lock is readable");
    let sources = std::fs::read_to_string(
        root.join("crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json"),
    )
    .expect("cargo-sources.json is readable");

    // Only packages carrying a checksum come from a registry; path and git
    // dependencies (the workspace crates themselves) are not vendored.
    let mut wanted: Vec<String> = Vec::new();
    for block in lock.split("[[package]]") {
        let field = |key: &str| -> Option<String> {
            block.lines().find_map(|line| {
                line.strip_prefix(&format!("{key} = \""))
                    .and_then(|rest| rest.strip_suffix('"'))
                    .map(str::to_string)
            })
        };
        if field("checksum").is_some() {
            if let (Some(name), Some(version)) = (field("name"), field("version")) {
                wanted.push(format!("cargo/vendor/{name}-{version}"));
            }
        }
    }
    assert!(
        wanted.len() > 100,
        "parsed only {} locked packages; the lockfile format probably changed",
        wanted.len()
    );

    let parsed: serde_json::Value =
        serde_json::from_str(&sources).expect("cargo-sources.json is valid JSON");
    let have: Vec<String> = parsed
        .as_array()
        .expect("cargo-sources.json is an array")
        .iter()
        .filter(|entry| entry.get("type").and_then(|t| t.as_str()) == Some("archive"))
        .filter_map(|entry| {
            entry
                .get("dest")
                .and_then(|d| d.as_str())
                .map(str::to_string)
        })
        .collect();

    let missing: Vec<&String> = wanted.iter().filter(|w| !have.contains(w)).collect();
    let stale: Vec<&String> = have.iter().filter(|h| !wanted.contains(h)).collect();

    assert!(
        missing.is_empty(),
        "locked crates with no vendor entry -- the flatpak build will fail on these:\n  {}\n\
         Regenerate: see the comment above `cargo-sources.json` in \
         crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.yml",
        missing
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
    assert!(
        stale.is_empty(),
        "vendor entries for crates no longer in the lockfile:\n  {}",
        stale
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}
