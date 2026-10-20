//! Reporting a Codex client version, so the ChatGPT backend does not hide
//! models the account is entitled to.
//!
//! The Codex backend gates newer models behind the `client_version` the caller
//! reports. A stale value does not fail — it silently returns a shorter model
//! list, which is the worst shape a bug can take: the user sees IronWire
//! offering fewer models than Codex does and has no way to tell why.
//!
//! Ported rather than delegated: `ironclaw_llm::codex_chatgpt` is a private
//! module, so there is nothing to call. The parsing rules and the fallback
//! constant follow it deliberately, so the two products report the same thing
//! (`docs/DESIGN.md` §7).

use std::time::Duration;

use tokio::sync::OnceCell;

/// Reported when the installed `codex` binary cannot be queried.
///
/// A last resort, not a default to rely on: it needs raising as Codex releases,
/// and the whole point of detection is that we usually do not reach it.
pub const DEFAULT_CLIENT_VERSION: &str = "0.137.0";

/// Environment override, for a user whose `codex` is not on IronWire's `PATH` —
/// a real case under a service manager, where the daemon's `PATH` is not the
/// user's shell `PATH`.
pub const CLIENT_VERSION_ENV: &str = "IRONWIRE_CODEX_CLIENT_VERSION";

/// The command is effectively instant. The bound only guards against a wedged
/// binary stalling a request.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Where the app installs on macOS, in the order a user is likely to have it.
///
/// A desktop-only user has no `codex` on `PATH` at all, so without this the
/// chain falls straight through to the compiled-in constant and hands them a
/// shortened model list with nothing to explain it.
const APP_BUNDLE_PLISTS: &[&str] = &[
    "/Applications/Codex.app/Contents/Info.plist",
    "~/Applications/Codex.app/Contents/Info.plist",
];

/// Where a reported version came from.
///
/// Carried rather than discarded because the failure this module exists to
/// prevent is invisible: a stale version returns a *shorter model list*, not an
/// error, so "which source won" is the only diagnostic there is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionSource {
    /// `IRONWIRE_CODEX_CLIENT_VERSION`.
    Environment,
    /// `codex --version`.
    Cli,
    /// The desktop app's `Info.plist`.
    App,
    /// Nothing answered; [`DEFAULT_CLIENT_VERSION`].
    Fallback,
}

impl VersionSource {
    /// One clause for `ironwire doctor`.
    #[must_use]
    pub fn describe(self) -> &'static str {
        match self {
            Self::Environment => "from IRONWIRE_CODEX_CLIENT_VERSION",
            Self::Cli => "from `codex --version`",
            Self::App => "from the Codex app bundle",
            Self::Fallback => {
                "compiled-in fallback — no Codex CLI or app found, so the model \
                 list may be shorter than your account allows"
            }
        }
    }
}

static DETECTED: OnceCell<String> = OnceCell::const_new();

/// The `client_version` to report, detected once per process.
///
/// Cached because it cannot change while we are running — the user would have
/// to upgrade Codex, and they can restart the daemon.
pub async fn client_version() -> String {
    DETECTED
        .get_or_init(|| async {
            let (version, source) = detect_with_source().await;
            tracing::debug!(%version, ?source, "reporting a Codex client version");
            version
        })
        .await
        .clone()
}

/// The version to report, and where it came from.
///
/// Ordered, first hit wins: an explicit override, then the CLI, then the
/// installed app, then the constant. Every miss is a normal state — plenty of
/// people run IronWire with neither Codex installed — so none of them is an
/// error, and only the last one is worth warning about.
pub async fn detect_with_source() -> (String, VersionSource) {
    if let Ok(override_value) = std::env::var(CLIENT_VERSION_ENV)
        && !override_value.trim().is_empty()
    {
        return (
            override_value.trim().to_string(),
            VersionSource::Environment,
        );
    }
    if let Some(version) = detect().await {
        return (version, VersionSource::Cli);
    }
    if let Some(version) = detect_app().await {
        return (version, VersionSource::App);
    }
    (DEFAULT_CLIENT_VERSION.to_string(), VersionSource::Fallback)
}

/// Read `CFBundleShortVersionString` from an installed Codex app bundle.
///
/// Deliberately a substring scan rather than a plist parser: the file is XML or
/// binary depending on how it was built, this is a diagnostic aid, and a
/// dependency on a plist crate to read one string would be a poor trade. A miss
/// is a normal state.
async fn detect_app() -> Option<String> {
    for path in APP_BUNDLE_PLISTS {
        let expanded = match path.strip_prefix("~/") {
            Some(rest) => dirs::home_dir()?.join(rest),
            None => std::path::PathBuf::from(path),
        };
        let Ok(bytes) = tokio::time::timeout(PROBE_TIMEOUT, tokio::fs::read(&expanded)).await
        else {
            continue;
        };
        let Ok(bytes) = bytes else { continue };
        if let Some(version) = parse_bundle_version(&String::from_utf8_lossy(&bytes)) {
            return Some(version);
        }
    }
    None
}

/// Pull `CFBundleShortVersionString`'s value out of a plist.
#[must_use]
pub fn parse_bundle_version(plist: &str) -> Option<String> {
    let key = plist.find("CFBundleShortVersionString")?;
    let after = &plist[key..];
    let open = after.find("<string>")? + "<string>".len();
    let close = after[open..].find("</string>")?;
    parse(&after[open..open + close])
}

/// Fall back without pretending we detected anything.
#[must_use]
pub fn resolve(detected: Option<&str>) -> String {
    detected
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(DEFAULT_CLIENT_VERSION)
        .to_string()
}

/// Ask the installed `codex` for its version.
///
/// `None` when the binary is absent, times out, exits non-zero, or prints
/// something we cannot parse. Every one of those is a normal state — plenty of
/// people run IronWire without Codex installed — so none of them is an error.
async fn detect() -> Option<String> {
    let output = tokio::time::timeout(
        PROBE_TIMEOUT,
        tokio::process::Command::new("codex")
            .arg("--version")
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    parse(&String::from_utf8_lossy(&output.stdout))
}

/// Pull the version token out of `codex --version` output.
///
/// Accepts `codex-cli 0.137.0`, `codex 0.140.1`, and pre-release or
/// build-metadata suffixes — `0.141.0-beta.2` resolves to `0.141.0`, because
/// what the backend gates on is the release number and a pre-release user
/// should not be handed a shorter model list than a stable one.
#[must_use]
pub fn parse(output: &str) -> Option<String> {
    output.split_whitespace().find_map(|token| {
        let core = token.split(['-', '+']).next().unwrap_or(token);
        let segments: Vec<&str> = core.split('.').collect();
        let is_version = segments.len() >= 2
            && segments
                .iter()
                .all(|seg| !seg.is_empty() && seg.bytes().all(|b| b.is_ascii_digit()));
        is_version.then(|| core.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bundle_version_is_read_from_either_plist_shape() {
        let xml = r#"<?xml version="1.0"?><plist><dict>
            <key>CFBundleName</key><string>Codex</string>
            <key>CFBundleShortVersionString</key><string>0.145.0</string>
            </dict></plist>"#;
        assert_eq!(parse_bundle_version(xml).as_deref(), Some("0.145.0"));
        assert_eq!(parse_bundle_version("no such key").as_deref(), None);
    }

    /// Every source missing is a normal state — plenty of people run IronWire
    /// with no Codex at all — but it is the one that silently shortens the
    /// model list, so it must be nameable.
    #[test]
    fn the_fallback_says_what_it_costs() {
        assert!(
            VersionSource::Fallback.describe().contains("shorter"),
            "the one source worth warning about does not say why"
        );
        assert!(VersionSource::Cli.describe().contains("codex --version"));
        assert!(VersionSource::App.describe().contains("app"));
    }

    #[test]
    fn the_common_output_shapes_parse() {
        assert_eq!(parse("codex-cli 0.137.0").as_deref(), Some("0.137.0"));
        assert_eq!(parse("codex 0.140.1").as_deref(), Some("0.140.1"));
        assert_eq!(parse("0.99.3\n").as_deref(), Some("0.99.3"));
    }

    #[test]
    fn a_prerelease_resolves_to_its_release_number() {
        // Someone on a beta build should not be handed a shorter model list
        // than someone on stable.
        assert_eq!(parse("codex 0.141.0-beta.2").as_deref(), Some("0.141.0"));
        assert_eq!(parse("codex 0.141.0+build.7").as_deref(), Some("0.141.0"));
    }

    #[test]
    fn output_with_no_version_yields_nothing_rather_than_a_guess() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("command not found"), None);
        // A single integer is not a version; treating "42" as one would report
        // nonsense to the backend.
        assert_eq!(parse("codex 42"), None);
    }

    #[test]
    fn a_missing_detection_falls_back_without_pretending() {
        assert_eq!(resolve(None), DEFAULT_CLIENT_VERSION);
        assert_eq!(resolve(Some("   ")), DEFAULT_CLIENT_VERSION);
        assert_eq!(resolve(Some("0.150.0")), "0.150.0");
    }

    #[tokio::test]
    async fn detection_never_panics_when_codex_is_absent() {
        // The common case for most IronWire users, and it must be quiet.
        let version = client_version().await;
        assert!(!version.is_empty());
    }
}
