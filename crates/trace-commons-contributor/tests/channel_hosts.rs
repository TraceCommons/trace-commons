//! The ratchet behind `src/channel.rs`: every compiled-in NEAR AI host, Trace
//! Commons host and app identity value lives in that one module, with one
//! value per channel (the `near-ai-staging` build feature).
//!
//! A host or identity written anywhere else is a value the feature does not
//! switch, so a staging build would carry a production host or the reverse.
//! These tests read the non-test source of the contributor, FFI and platform
//! crates and fail naming each such literal. A reintroduced one fails here,
//! not on a tester's machine.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use trace_commons_contributor::channel;
#[cfg(not(feature = "near-ai-staging"))]
use trace_commons_contributor::config::{
    CONTRIBUTOR_CONFIG_SCHEMA_VERSION, ConfigStore, ContributorConfig,
};

/// The crates whose source ships in the contributor daemon, the FFI dylib
/// and the bundled `near-ai` CLI.
const SCANNED: [&str; 3] = [
    "src",
    "../trace-commons-contributor-ffi/src",
    "../trace-commons-contributor-platform/src",
];

/// The one file allowed to hold host and identity literals.
const CHANNEL_MODULE: &str = "src/channel.rs";

/// Literals that look like a host but are not one this build contacts or
/// trusts, with the reason. Each entry names the file and the literal, so a
/// new literal in the same file is still caught.
const NOT_A_HOST: [(&str, &str, &str); 2] = [
    (
        "src/skill_loop/evaluation/fixtures/catalog.rs",
        "PublisherUrl: https://tracecommons.ai",
        "text of a package-manifest fixture shown to a model under evaluation",
    ),
    (
        "src/skill_loop/evaluation/fixtures/catalog.rs",
        "PackageUrl: https://tracecommons.ai/install/",
        "text of a package-manifest fixture shown to a model under evaluation",
    ),
];

/// A NEAR AI or Trace Commons domain at the end of a host name.
const HOST_DOMAINS: [&str; 2] = ["near.ai", "tracecommons.ai"];

/// Identity values: the keychain team group and bundle id, the keychain
/// service namespace, the state directory's name and its override variable.
fn is_identity_literal(literal: &str) -> bool {
    literal.contains("KXSWJN7WY8")
        || literal.contains("ai.tracecommons.shell")
        || literal.starts_with("trace-commons.")
        || (literal.starts_with("trace-commons/") && !is_domain_separator(literal))
        || literal.starts_with("trace-commons-staging")
        || literal == "trace-commons"
        || literal.contains("TRACE_COMMONS_CONTRIBUTOR_DIR")
        || literal.contains("TRACE_COMMONS_STAGING_CONTRIBUTOR_DIR")
}

/// A versioned hash or binding domain such as `trace-commons/device-key/v1`:
/// a label inside a digest, not a path, and changing it would orphan every
/// record it binds.
fn is_domain_separator(literal: &str) -> bool {
    literal
        .trim_end_matches("\\0")
        .rsplit('/')
        .next()
        .and_then(|segment| segment.strip_prefix('v'))
        .is_some_and(|version| !version.is_empty() && version.bytes().all(|b| b.is_ascii_digit()))
}

/// The hosts named in `literal`: a domain above preceded by the start of the
/// literal, a `/` (after a scheme) or `@`. Prose such as "Sign in with
/// near.ai" is a product name, not a host, and is not matched.
fn hosts_in(literal: &str) -> Vec<String> {
    let mut hosts = Vec::new();
    for domain in HOST_DOMAINS {
        let mut from = 0;
        while let Some(found) = literal[from..].find(domain) {
            let end = from + found + domain.len();
            let after_ok = literal[end..]
                .chars()
                .next()
                .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '-'));
            let mut start = from + found;
            while start > 0 {
                let c = literal.as_bytes()[start - 1] as char;
                if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                    start -= 1;
                } else {
                    break;
                }
            }
            let before_ok = start == 0 || matches!(literal.as_bytes()[start - 1], b'/' | b'@');
            if after_ok && before_ok {
                hosts.push(literal[start..end].to_string());
            }
            from = end;
        }
    }
    hosts
}

/// The string literals of `source` outside comments and outside any item
/// under `#[cfg(test)]`, each with its line number.
fn non_test_literals(source: &str) -> Vec<(usize, String)> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut line = 1;
    let mut i = 0;
    // Depth of `{` and the depth at which a skipped test item started.
    let mut depth = 0usize;
    let mut skipping: Option<usize> = None;
    let mut pending_test_item = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            i += 1;
            continue;
        }
        if chars[i..].starts_with(&['/', '/']) {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if chars[i..].starts_with(&['/', '*']) {
            let mut nest = 0;
            loop {
                assert!(i < chars.len(), "unterminated block comment");
                if chars[i..].starts_with(&['/', '*']) {
                    nest += 1;
                    i += 2;
                } else if chars[i..].starts_with(&['*', '/']) {
                    nest -= 1;
                    i += 2;
                    if nest == 0 {
                        break;
                    }
                } else {
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
            continue;
        }
        if c == '#' && chars.get(i + 1) == Some(&'[') {
            let close = (i..chars.len()).find(|&j| chars[j] == ']').unwrap();
            let attribute: String = chars[i..=close].iter().collect();
            let compact: String = attribute.chars().filter(|c| !c.is_whitespace()).collect();
            if compact == "#[cfg(test)]" || compact.starts_with("#[cfg(all(test,") {
                pending_test_item = true;
            }
            i = close + 1;
            continue;
        }
        // Raw strings, byte strings and plain strings.
        let mut start = i;
        if chars[start] == 'b' && matches!(chars.get(start + 1), Some('r') | Some('"')) {
            start += 1;
        }
        let is_raw = chars[start] == 'r'
            && (chars.get(start + 1) == Some(&'"') || chars.get(start + 1) == Some(&'#'))
            && (i == 0 || !(chars[i - 1].is_ascii_alphanumeric() || chars[i - 1] == '_'));
        if is_raw {
            let mut quote = start + 1;
            while chars.get(quote) == Some(&'#') {
                quote += 1;
            }
            if chars.get(quote) == Some(&'"') {
                let hashes = quote - start - 1;
                let first_line = line;
                let mut j = quote + 1;
                let mut text = String::new();
                loop {
                    assert!(j < chars.len(), "unterminated raw string");
                    if chars[j] == '"' && (0..hashes).all(|n| chars.get(j + 1 + n) == Some(&'#')) {
                        j += 1 + hashes;
                        break;
                    }
                    if chars[j] == '\n' {
                        line += 1;
                    }
                    text.push(chars[j]);
                    j += 1;
                }
                if skipping.is_none() && !pending_test_item {
                    out.push((first_line, text));
                }
                i = j;
                continue;
            }
        }
        if chars[start] == '"' && (start == i || chars[i] == 'b') {
            let first_line = line;
            let mut j = start + 1;
            let mut text = String::new();
            loop {
                assert!(j < chars.len(), "unterminated string");
                if chars[j] == '\\' {
                    if chars.get(j + 1) == Some(&'\n') {
                        line += 1;
                    }
                    text.push(chars[j]);
                    text.push(chars[j + 1]);
                    j += 2;
                    continue;
                }
                if chars[j] == '"' {
                    j += 1;
                    break;
                }
                if chars[j] == '\n' {
                    line += 1;
                }
                text.push(chars[j]);
                j += 1;
            }
            if skipping.is_none() && !pending_test_item {
                out.push((first_line, text));
            }
            i = j;
            continue;
        }
        // Character literals, so `'{'` is not counted as a brace. A lifetime
        // (`'a`) has no closing quote two characters on.
        if c == '\'' {
            if chars.get(i + 1) == Some(&'\\') {
                let close = (i + 2..chars.len()).find(|&j| chars[j] == '\'').unwrap();
                i = close + 1;
                continue;
            }
            if chars.get(i + 2) == Some(&'\'') {
                i += 3;
                continue;
            }
        }
        match c {
            '{' => {
                if pending_test_item && skipping.is_none() {
                    skipping = Some(depth);
                    pending_test_item = false;
                }
                depth += 1;
            }
            '}' => {
                depth -= 1;
                if skipping == Some(depth) {
                    skipping = None;
                }
            }
            ';' if pending_test_item && skipping.is_none() => {
                // A test-only item without a body: `mod tests;`, a `use`, a
                // `static` or `const`.
                pending_test_item = false;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// Every source file under the scanned crates that may compile into a build:
/// `.rs` files, and `.js` served by the daemon. Test-only files are left out
/// by their names, which `#[cfg(test)] #[path = ...]` modules use.
fn source_files() -> Vec<(String, PathBuf)> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for root in SCANNED {
        let mut stack = vec![manifest.join(root)];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("source directory is readable") {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                let test_only = name == "tests.rs"
                    || name.ends_with("_tests.rs")
                    || name.ends_with("_test.rs")
                    || path.components().any(|c| c.as_os_str() == "tests");
                let compiled = name.ends_with(".rs") || name.ends_with(".js");
                if compiled && !test_only {
                    let relative = path
                        .strip_prefix(manifest)
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| {
                            // The sibling crates: name them from `crates/`.
                            let crates = manifest.parent().unwrap();
                            format!(
                                "../{}",
                                path.strip_prefix(crates).unwrap().to_string_lossy()
                            )
                        });
                    // Name files with `/` on every platform, so the
                    // module and allowance paths above match on Windows.
                    let relative = relative.replace('\\', "/");
                    files.push((relative, path));
                }
            }
        }
    }
    files.sort();
    files
}

/// Each literal outside the channel module that names a host or identity
/// value, as `file:line: literal`.
fn literals_outside_the_channel_module() -> Vec<String> {
    let mut found = Vec::new();
    for (relative, path) in source_files() {
        if relative == CHANNEL_MODULE {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        let literals = if relative.ends_with(".js") {
            source
                .lines()
                .enumerate()
                .filter(|(_, line)| !line.trim_start().starts_with("//"))
                .flat_map(|(n, line)| {
                    // Double-quoted, single-quoted and template literals. A
                    // stray apostrophe in prose yields a span that is not a
                    // literal, which can only add a finding, never hide one.
                    ['"', '\'', '`']
                        .into_iter()
                        .flat_map(|quote| {
                            line.split(quote)
                                .skip(1)
                                .step_by(2)
                                .map(str::to_string)
                                .collect::<Vec<_>>()
                        })
                        .map(move |literal| (n + 1, literal))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        } else {
            non_test_literals(&source)
        };
        for (line, literal) in literals {
            let named = !hosts_in(&literal).is_empty() || is_identity_literal(&literal);
            let excused = NOT_A_HOST
                .iter()
                .any(|(file, text, _)| *file == relative && literal.contains(text));
            if named && !excused {
                found.push(format!("{relative}:{line}: {literal:?}"));
            }
        }
    }
    found
}

/// The scan itself works: it sees a host literal, skips comments, prose and
/// test items. Without this a scanner that matched nothing would pass.
#[test]
fn the_scan_finds_a_host_and_skips_tests_comments_and_prose() {
    let source = r##"
// "https://cloud-api.near.ai" in a comment
const A: &str = "https://cloud-api.near.ai/v1";
const B: &str = "Sign in with near.ai";
const C: &str = "cloud.near.ai";
const D: &str = r#"https://ingest.tracecommons.ai"#;
const E: char = '{';
#[cfg(test)]
mod tests {
    const F: &str = "https://evil.tracecommons.ai";
    fn g() { let _ = "{"; }
}
const H: &str = "KXSWJN7WY8.ai.tracecommons.shell";
const I: &str = "trace-commons/mission-drafts";
const J: &[u8] = b"trace-commons/device-key/v1\0";
"##;
    let literals = non_test_literals(source);
    let named: Vec<String> = literals
        .iter()
        .filter(|(_, l)| !hosts_in(l).is_empty() || is_identity_literal(l))
        .map(|(_, l)| l.clone())
        .collect();
    assert_eq!(
        named,
        [
            "https://cloud-api.near.ai/v1",
            "cloud.near.ai",
            "https://ingest.tracecommons.ai",
            "KXSWJN7WY8.ai.tracecommons.shell",
            "trace-commons/mission-drafts",
        ]
    );
    assert_eq!(
        hosts_in("https://cloud-api.near.ai/v1"),
        ["cloud-api.near.ai"]
    );
    assert!(hosts_in("Opening near.ai in your browser").is_empty());
}

fn assert_no_literal_outside_the_channel_module() {
    let found = literals_outside_the_channel_module();
    assert!(
        found.is_empty(),
        "move these into src/channel.rs:\n{}",
        found.join("\n")
    );
}

/// Every host this build's channel values name.
fn channel_hosts() -> Vec<String> {
    channel::HOST_VALUES
        .iter()
        .flat_map(|value| hosts_in(value))
        .collect()
}

fn digest(host: &str) -> String {
    hex::encode(Sha256::digest(host.as_bytes()))
}

/// A staging build: no host or identity literal outside the channel module,
/// and every host the channel module gives this build is a staging host.
#[cfg(feature = "near-ai-staging")]
#[test]
fn a_staging_build_names_no_production_host() {
    assert_no_literal_outside_the_channel_module();
    assert_eq!(channel::CHANNEL, "staging");
    let hosts = channel_hosts();
    assert!(!hosts.is_empty());
    for host in hosts {
        let staging_site =
            host == channel::STAGING_HOST || host.ends_with(&format!(".{}", channel::STAGING_HOST));
        let near_ai_staging = host.ends_with(".near.ai") && host.contains("-stg");
        assert!(
            staging_site || near_ai_staging,
            "a staging build names {host}, which is not a staging host"
        );
    }
    // Identity follows the channel too.
    assert!(channel::STATE_DIR_NAME.ends_with("-staging"));
    assert!(channel::STATE_DIR_ENV.contains("STAGING"));
    assert!(channel::URL_SCHEME.ends_with("-staging"));
    assert!(channel::KEYCHAIN_ACCESS_GROUP.ends_with(".staging"));
    for service in channel::KEYCHAIN.services() {
        assert!(service.starts_with("trace-commons-staging."), "{service}");
    }
}

/// The deny list a production build holds is digests, and they are the
/// digests of the staging hosts: checked from the staging side, where the
/// plaintext exists, so the list cannot rot.
#[cfg(feature = "near-ai-staging")]
#[test]
fn the_production_deny_list_is_the_digests_of_the_staging_hosts() {
    let mut staging: Vec<String> = channel_hosts()
        .into_iter()
        .filter(|host| !host.ends_with(&format!(".{}", channel::STAGING_HOST)))
        .map(|host| digest(&host))
        .collect();
    staging.sort();
    staging.dedup();
    let mut listed: Vec<String> = channel::STAGING_HOST_DIGESTS
        .iter()
        .map(|d| d.to_string())
        .collect();
    listed.sort();
    assert_eq!(listed, staging);
}

/// A default build: no host or identity literal outside the channel module,
/// no staging host among its values, and the staging hosts it refuses are
/// held only as SHA-256 digests.
#[cfg(not(feature = "near-ai-staging"))]
#[test]
fn a_default_build_names_no_staging_host() {
    assert_no_literal_outside_the_channel_module();
    assert_eq!(channel::CHANNEL, "production");
    let hosts = channel_hosts();
    assert!(!hosts.is_empty());
    for host in &hosts {
        assert!(
            !host.contains("staging") && !host.contains("-stg"),
            "a default build names {host}"
        );
        assert!(
            !channel::STAGING_HOST_DIGESTS.contains(&digest(host).as_str()),
            "{host} is both a value of this build and on its deny list"
        );
    }
    for value in channel::HOST_VALUES {
        assert!(
            !value.contains("staging") && !value.contains("-stg"),
            "{value}"
        );
    }
    assert!(!channel::STAGING_HOST_DIGESTS.is_empty());
    for entry in channel::STAGING_HOST_DIGESTS {
        assert_eq!(entry.len(), 64, "{entry} is not a SHA-256 digest");
        assert!(entry.bytes().all(|b| b.is_ascii_hexdigit()));
    }
}

/// Sign-in, inference, receipts and the attestation report are one NEAR AI
/// host in each channel, so a key minted by one is only ever sent to it.
#[test]
fn every_near_ai_host_in_a_build_is_one_host() {
    let host = |url: &str| {
        url::Url::parse(url)
            .unwrap()
            .host_str()
            .unwrap()
            .to_string()
    };
    let api = host(channel::NEAR_AI_API_ORIGIN);
    assert_eq!(host(channel::NEAR_AI_INFERENCE_BASE), api);
    assert_eq!(host(channel::NEAR_AI_ATTESTATION_GATEWAY), api);
    assert_eq!(host(channel::NEAR_AI_PRIVACY_FILTER_BASE), api);
    // The dashboard and the wallet recipient are the console host, a
    // different name, but the same channel's.
    assert_eq!(
        host(channel::NEAR_AI_DASHBOARD_ORIGIN),
        channel::NEAR_AI_WALLET_RECIPIENT
    );
    assert_eq!(
        channel::NEAR_AI_INFERENCE_BASE,
        format!("{}/v1", channel::NEAR_AI_API_ORIGIN)
    );
}

/// A production build refuses an ingest or issuer origin on the staging host,
/// whether it arrives in an invite or a hand-edited config.
#[cfg(not(feature = "near-ai-staging"))]
#[test]
fn a_production_build_refuses_a_staging_ingest_origin() {
    // The digest of the staging site, spelled here only as text the test
    // builds, never as a constant in the crate.
    let staging_site = ["staging", "tracecommons", "ai"].join(".");
    for origin in [
        format!("https://ingest.{staging_site}"),
        format!("https://issuer.{staging_site}/onboard"),
        format!("https://{staging_site}"),
        format!("https://INGEST.{}", staging_site.to_uppercase()),
        "https://cloud-stg-api.near.ai/v1".to_string(),
    ] {
        assert!(
            channel::refuse_other_channel_origin(&origin).is_err(),
            "{origin} was accepted"
        );
    }
    for origin in [
        channel::NATIVE_ACCOUNT_ORIGIN,
        "https://issuer.tracecommons.ai",
        "https://ingest.example",
        "http://127.0.0.1:3907",
    ] {
        channel::refuse_other_channel_origin(origin).expect(origin);
    }

    // And the config store will not persist or load one. Positive control
    // first: the same config on an ordinary host round-trips.
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let accepted = config("https://ingest.example");
    store.save_config(&accepted).unwrap();
    assert_eq!(
        store.load_config().unwrap().map(|c| c.ingest_url),
        Some(accepted.ingest_url)
    );
    let refused = config(&format!("https://ingest.{staging_site}"));
    assert!(store.save_config(&refused).is_err());
    let path = dir.path().join("state").join("contributor.json");
    let mut raw: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    raw["ingest_url"] = serde_json::Value::String(refused.ingest_url.clone());
    std::fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
    let error = store.load_config().unwrap_err();
    assert_eq!(error.to_string(), "service-origin-other-channel");
}

#[cfg(not(feature = "near-ai-staging"))]
fn config(ingest_url: &str) -> ContributorConfig {
    ContributorConfig {
        inference_receipt_endpoint: None,
        consent_scopes_chosen: Some(true),
        witness_origin: None,
        inference_receipt_check_attestation: false,
        schema_version: CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
        issuer_url: "https://issuer.example".into(),
        ingest_url: ingest_url.into(),
        audience: "trace-commons-upload".into(),
        tenant_id: "tenant-abc".into(),
        instance_id: "instance-1".into(),
        user_subject: "alice".into(),
        device_key_id: "sha256:00".into(),
        consent_scopes: vec!["debugging_evaluation".into()],
        pii_filter: None,
        allowed_hosts: None,
        display_handle: None,
        public_bio: None,
        public_since: None,
        witness: None,
    }
}

/// A staging build refuses the production hosts the same way.
#[cfg(feature = "near-ai-staging")]
#[test]
fn a_staging_build_refuses_a_production_ingest_origin() {
    for origin in [
        "https://ingest.tracecommons.ai",
        "https://issuer.tracecommons.ai/onboard",
        "https://tracecommons.ai",
        "https://www.tracecommons.ai",
        "https://cloud-api.near.ai/v1",
        "https://cloud.near.ai",
    ] {
        assert!(
            channel::refuse_other_channel_origin(origin).is_err(),
            "{origin} was accepted"
        );
    }
    channel::refuse_other_channel_origin(channel::NATIVE_ACCOUNT_ORIGIN).unwrap();
    channel::refuse_other_channel_origin("https://ingest.example").unwrap();
}

/// The keychain group, the associated domain and the application id each
/// channel's binary asks for are the ones its entitlements file grants. The
/// production file is checked in a default build, the staging file in a
/// staging build. If either drifts, the signature is still valid and the
/// first thing to notice is a launch.
#[test]
fn the_staging_entitlements_grant_the_staging_access_group() {
    let file = if cfg!(feature = "near-ai-staging") {
        "entitlements-staging.plist"
    } else {
        "entitlements.plist"
    };
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../macos")
        .join(file);
    let entitlements = std::fs::read_to_string(&path).expect("entitlements file exists");
    let array_after = |key: &str| -> String {
        let after = entitlements
            .split_once(&format!("<key>{key}</key>"))
            .unwrap_or_else(|| panic!("{file} declares {key}"))
            .1;
        after
            .split_once("<array>")
            .unwrap()
            .1
            .split_once("</array>")
            .unwrap()
            .0
            .to_string()
    };
    assert!(
        array_after("keychain-access-groups").contains(&format!(
            "<string>{}</string>",
            channel::KEYCHAIN_ACCESS_GROUP
        )),
        "{file} must grant {}",
        channel::KEYCHAIN_ACCESS_GROUP
    );
    assert!(
        array_after("com.apple.developer.associated-domains").contains(&format!(
            "<string>webcredentials:{}</string>",
            channel::PASSKEY_RP_ID
        )),
        "{file} must name webcredentials:{}",
        channel::PASSKEY_RP_ID
    );
    let application_id = entitlements
        .split_once("<key>com.apple.application-identifier</key>")
        .unwrap()
        .1
        .split_once("</string>")
        .unwrap()
        .0
        .trim()
        .trim_start_matches("<string>")
        .to_string();
    assert_eq!(application_id, channel::KEYCHAIN_ACCESS_GROUP);
}

/// The staging entitlements are production's with exactly three lines
/// changed: the application id, the keychain access group and the
/// `webcredentials:` domain, each the staging twin of production's. Anything
/// else (a sandbox, a network or hardened-runtime entitlement) is the same
/// in both, so a staging build is tested under production's entitlements.
#[test]
fn the_staging_entitlements_differ_from_production_in_three_lines() {
    let read = |file: &str| {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../macos")
                .join(file),
        )
        .expect("entitlements file exists")
    };
    let production = read("entitlements.plist");
    let staging = read("entitlements-staging.plist");
    let production: Vec<&str> = production.lines().collect();
    let staging: Vec<&str> = staging.lines().collect();
    assert_eq!(production.len(), staging.len());
    let changed: Vec<(&str, &str)> = production
        .iter()
        .zip(&staging)
        .filter(|(p, s)| p != s)
        .map(|(p, s)| (p.trim(), s.trim()))
        .collect();
    let group = "<string>KXSWJN7WY8.ai.tracecommons.shell";
    assert_eq!(
        changed,
        [
            (
                format!("{group}</string>").as_str(),
                format!("{group}.staging</string>").as_str()
            ),
            (
                format!("{group}</string>").as_str(),
                format!("{group}.staging</string>").as_str()
            ),
            (
                "<string>webcredentials:tracecommons.ai</string>",
                "<string>webcredentials:staging.tracecommons.ai</string>"
            ),
        ]
    );
}

/// IronWire falls back to its own compiled-in NEAR AI base, which is
/// production, whenever nothing names one. Every place this crate writes an
/// IronWire NEAR AI backend names the base, and the daemon's private-inference
/// proxy, which runs on the person's own IronWire home, is only handed a NEAR
/// AI key when that home's NEAR AI base is this channel's.
#[test]
fn ironwire_is_never_built_on_its_default_base() {
    // Every non-test configuration that declares a NEAR AI backend gives it
    // a base_url, and the base it gives is the channel's.
    for (relative, path) in source_files() {
        if !relative.ends_with(".rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        for (line, literal) in non_test_literals(&source) {
            if literal.contains("kind = \"nearai\"") || literal.contains("kind = 'nearai'") {
                assert!(
                    literal.contains("base_url"),
                    "{relative}:{line} declares an IronWire NEAR AI backend with no base_url"
                );
            }
        }
    }
    let route =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/managed/route.rs"))
            .unwrap();
    assert!(route.contains("channel::NEAR_AI_INFERENCE_BASE"));

    use trace_commons_contributor::daemon::private_inference::nearai_key_may_go_to;
    let home = tempfile::tempdir().unwrap();
    // No configuration: IronWire would use its default, production.
    assert_eq!(
        nearai_key_may_go_to(home.path(), None),
        !cfg!(feature = "near-ai-staging")
    );
    // The channel's own base, declared in the home's configuration.
    std::fs::write(
        home.path().join("config.toml"),
        format!(
            "[[backends]]\nid = \"nearai\"\nkind = \"nearai\"\nbase_url = \"{}\"\n",
            channel::NEAR_AI_INFERENCE_BASE
        ),
    )
    .unwrap();
    assert!(nearai_key_may_go_to(home.path(), None));
    // The channel's own base, from IronWire's environment override.
    let elsewhere = tempfile::tempdir().unwrap();
    assert!(nearai_key_may_go_to(
        elsewhere.path(),
        Some(channel::NEAR_AI_INFERENCE_BASE)
    ));
    // Another channel's base: never in a staging build.
    std::fs::write(
        home.path().join("config.toml"),
        "[[backends]]\nid = \"nearai\"\nkind = \"nearai\"\nbase_url = \"https://other.example/v1\"\n",
    )
    .unwrap();
    assert_eq!(
        nearai_key_may_go_to(home.path(), None),
        !cfg!(feature = "near-ai-staging")
    );
    // An unreadable configuration: never in a staging build.
    std::fs::write(home.path().join("config.toml"), "[[backends").unwrap();
    assert_eq!(
        nearai_key_may_go_to(home.path(), None),
        !cfg!(feature = "near-ai-staging")
    );

    // A second NEAR AI entry under another id gets the same key, and IronWire
    // builds it on its own base_url alone, with no environment fallback.
    let own = channel::NEAR_AI_INFERENCE_BASE;
    let two_entries = |second: &str| {
        format!(
            "[[backends]]\nid = \"nearai\"\nkind = \"nearai\"\nbase_url = \"{own}\"\n\
             [[backends]]\nid = \"nearai-team\"\nkind = \"nearai\"\n{second}"
        )
    };
    // With no base_url it is IronWire's default, production: never in a
    // staging build, whatever the environment says.
    std::fs::write(home.path().join("config.toml"), two_entries("")).unwrap();
    assert_eq!(
        nearai_key_may_go_to(home.path(), Some(own)),
        !cfg!(feature = "near-ai-staging")
    );
    // On the channel's own base: allowed.
    std::fs::write(
        home.path().join("config.toml"),
        two_entries(&format!("base_url = \"{own}\"\n")),
    )
    .unwrap();
    assert!(nearai_key_may_go_to(home.path(), None));
    // On the other channel's base: never, in either build.
    std::fs::write(
        home.path().join("config.toml"),
        two_entries(&format!(
            "base_url = \"{}\"\n",
            other_channel_near_ai_base()
        )),
    )
    .unwrap();
    assert!(!nearai_key_may_go_to(home.path(), None));

    // The other channel's base as the `nearai` entry's, or from the
    // environment: never, in either build.
    std::fs::write(
        home.path().join("config.toml"),
        format!(
            "[[backends]]\nid = \"nearai\"\nkind = \"nearai\"\nbase_url = \"{}\"\n",
            other_channel_near_ai_base()
        ),
    )
    .unwrap();
    assert!(!nearai_key_may_go_to(home.path(), None));
    let empty = tempfile::tempdir().unwrap();
    assert!(!nearai_key_may_go_to(
        empty.path(),
        Some(&other_channel_near_ai_base())
    ));
}

/// The other channel's NEAR AI inference base.
fn other_channel_near_ai_base() -> String {
    if cfg!(feature = "near-ai-staging") {
        "https://cloud-api.near.ai/v1".to_string()
    } else {
        "https://cloud-stg-api.near.ai/v1".to_string()
    }
}

/// The protocol crate's privacy-filter default is production for the server;
/// the contributor never falls through to it.
#[test]
fn the_contributor_never_uses_the_protocol_privacy_filter_default() {
    for (relative, path) in source_files() {
        let source = std::fs::read_to_string(&path).unwrap();
        for forbidden in ["privacy_filter_near_ai::DEFAULT_BASE_URL", "build_from_env"] {
            assert!(
                !source.contains(forbidden),
                "{relative} uses {forbidden}, whose NEAR AI host is not the channel's"
            );
        }
    }
}

/// The Insights store a host opens when it names no directory is under the
/// channel's state directory name, so the staging and production apps on one
/// machine keep separate stores (each with its own channel's digest key).
/// Resolving it reads no state and creates none.
#[test]
fn the_default_insights_store_is_the_channel_s() {
    use trace_commons_contributor::insights::service;
    let dir = service::default_store_dir().expect("a per-user data directory");
    assert!(
        dir.ends_with(Path::new(channel::STATE_DIR_NAME).join("insights")),
        "{}",
        dir.display()
    );
    assert_eq!(service::STORE_CHANNEL.keychain, channel::KEYCHAIN);
    assert_eq!(
        service::STORE_CHANNEL.state_dir_name,
        channel::STATE_DIR_NAME
    );
}

/// The production identity values, exactly. Every installed production app
/// holds its keychain items under these service names and access group and
/// its state under this directory name and override variable, so changing
/// one strands every existing install: its enrollment, credentials and
/// Insights key are still there, under names the new build no longer reads.
/// Change them only with a migration.
#[cfg(not(feature = "near-ai-staging"))]
#[test]
fn the_production_identity_values_never_change() {
    assert_eq!(channel::STATE_DIR_NAME, "trace-commons");
    assert_eq!(channel::STATE_DIR_ENV, "TRACE_COMMONS_CONTRIBUTOR_DIR");
    assert_eq!(channel::URL_SCHEME, "tracecommons");
    assert_eq!(
        channel::KEYCHAIN,
        trace_commons_contributor_platform::os_secret_store::KeychainIdentity {
            access_group: "KXSWJN7WY8.ai.tracecommons.shell",
            cloud_service: "trace-commons.near-ai.credentials",
            account_service: "trace-commons.account.credentials",
            insights_digest_key_service: "trace-commons.insights.digest-key",
            claude_usage_service: "trace-commons.claude-usage.token-cache",
        }
    );
    assert_eq!(
        channel::KEYCHAIN_ACCESS_GROUP,
        channel::KEYCHAIN.access_group
    );
    assert_eq!(channel::PASSKEY_RP_ID, "tracecommons.ai");
    assert_eq!(channel::NEAR_AI_WALLET_RECIPIENT, "cloud.near.ai");
}

/// The override variable is checked where the CLI resolves its state
/// directory: naming the other channel's default directory is refused with a
/// label and no path, and nothing is created there. Naming this channel's own
/// default, whose name shares a prefix with the other's, is not refused.
#[cfg(unix)]
#[test]
fn an_override_to_the_other_channel_directory_is_refused_before_it_is_opened() {
    let home = tempfile::tempdir().unwrap();
    let home_path = home.path().canonicalize().unwrap();
    let config_base = if cfg!(target_os = "macos") {
        home_path.join("Library").join("Application Support")
    } else {
        home_path.join(".config")
    };
    let run = |dir: &Path| {
        std::process::Command::new(env!("CARGO_BIN_EXE_trace-commons-contributor"))
            .arg("whoami")
            .env("HOME", &home_path)
            .env("XDG_CONFIG_HOME", &config_base)
            .env(channel::STATE_DIR_ENV, dir)
            .output()
            .unwrap()
    };

    let other = config_base.join(channel::OTHER_STATE_DIR_NAME);
    let refused = run(&other.join("nested"));
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(!refused.status.success(), "{stderr}");
    assert!(stderr.contains("state-dir-other-channel"), "{stderr}");
    assert!(!stderr.contains(&*home_path.to_string_lossy()), "{stderr}");
    assert!(!other.exists(), "the refused directory was created");

    let own = config_base.join(channel::STATE_DIR_NAME);
    let allowed = run(&own);
    let stderr = String::from_utf8_lossy(&allowed.stderr);
    assert!(!stderr.contains("state-dir-other-channel"), "{stderr}");
    assert!(own.exists());
}
