//! `ironwire privacy check` — show what the filter would and would not catch.
//!
//! `docs/PRIVACY.md` §7 is the reason this exists. The mechanism has a
//! false-negative rate we cannot measure on a user's own data, so the honest
//! move is to let them measure it themselves, on a file they already
//! understand, before they decide how much to trust it.
//!
//! It deliberately prints **what matched**, not a verdict. There is no "clean"
//! and no checkmark: a file with no matches means this configuration found
//! nothing in it, which is a different statement from "this file is safe to
//! send".

use std::path::PathBuf;

use anyhow::{Context, Result};
use ironwire_core::config::Config;
use ironwire_core::config::PrivacyMode;
use ironwire_privacy::{Detector, Tiers};

use super::paths;

/// Run `ironwire privacy <action>`.
pub(crate) fn run(action: &str, path: Option<PathBuf>, mode: Option<String>) -> Result<()> {
    let mode_override = mode
        .as_deref()
        .map(|name| match name {
            "off" => Ok(PrivacyMode::Off),
            "credentials" => Ok(PrivacyMode::Credentials),
            "pii" => Ok(PrivacyMode::Pii),
            "full" => Ok(PrivacyMode::Full),
            other => anyhow::bail!("unknown mode `{other}` (try: off, credentials, pii, full)"),
        })
        .transpose()?;
    match action {
        "check" => {
            let path = path.context("usage: ironwire privacy check <file>")?;
            check(&path, mode_override)
        }
        "status" => status(),
        other => anyhow::bail!("unknown action `{other}` (try: check, status)"),
    }
}

fn load() -> Result<ironwire_core::config::PrivacyConfig> {
    let paths = paths()?;
    Ok(Config::load(&paths).context("loading config.toml")?.privacy)
}

fn status() -> Result<()> {
    let config = load()?;
    let mode = config.mode();
    println!("Privacy filter: {}", config.summary());
    println!("  {}", mode.describe());
    println!();
    if mode == PrivacyMode::Off {
        println!("Turn it on in $IRONWIRE_HOME/config.toml:");
        println!();
        println!("    [privacy]");
        println!("    mode = \"credentials\"   # or \"pii\", or \"full\"");
        println!("    named_values = [\"your-employer\", \"a-customer-domain.com\"]");
        println!();
        println!("Then restart the daemon. See `ironwire privacy check <file>` to");
        println!("see what that configuration would actually catch.");
        return Ok(());
    }

    println!("Exemptions (values inside these are left alone):");
    println!(
        "  fenced code blocks: {}",
        if config.scan_code_blocks {
            "scanned"
        } else {
            "exempt"
        }
    );
    println!(
        "  tool results:       {}",
        if config.scan_tool_results {
            "scanned"
        } else {
            "exempt"
        }
    );
    println!();
    println!("This reduces what reaches a provider. It is not a guarantee, and");
    println!("nothing here can tell you what it missed — run");
    println!("`ironwire privacy check <file>` on something you know well.");
    Ok(())
}

fn check(path: &std::path::Path, mode_override: Option<PrivacyMode>) -> Result<()> {
    let config = load()?;
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

    // `--mode` lets someone see what a level would catch *before* switching to
    // it, which is the only honest way to decide whether to trust it.
    let mode = mode_override.unwrap_or_else(|| config.mode());
    let detector = Detector::new(&Tiers {
        secrets: config.secrets || mode >= PrivacyMode::Pii,
        named_values: config.named_values.clone(),
        pii: mode >= PrivacyMode::Pii,
    });
    let findings = detector.find(&text);

    println!("{} — {} bytes", path.display(), text.len());
    match mode_override {
        Some(mode) => println!("Mode: {mode:?} (asked for, not what is configured)"),
        None => println!("Configuration: {}", config.summary()),
    }
    println!();

    if findings.is_empty() {
        // Deliberately not "clean". This configuration found nothing in this
        // file, which is a different claim.
        println!("No matches.");
        println!();
        println!("That means this configuration found nothing here — not that the");
        println!("file is safe to send. Anything it does not have a rule for, it");
        println!("cannot see.");
        return Ok(());
    }

    // Grouped by class so the false-positive rate of each is visible on its
    // own: a user deciding whether to trust `pii` needs to see how the phone
    // rule behaves separately from the email one.
    let mut by_class: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for finding in &findings {
        *by_class
            .entry(format!("{:?}", finding.class).to_lowercase())
            .or_default() += 1;
    }
    println!("{} match(es):", findings.len());
    println!(
        "  by class: {}",
        by_class
            .iter()
            .map(|(class, count)| format!("{class} {count}"))
            .collect::<Vec<_>>()
            .join(" · ")
    );
    println!();
    for finding in &findings {
        let line = text[..finding.range.start].lines().count().max(1);
        let matched = &text[finding.range.clone()];
        println!(
            "  {}:{line}  {}  {}",
            path.display(),
            finding.rule,
            preview(matched)
        );
    }
    println!();
    println!("Each of these would be replaced on the way out and restored on the");
    println!("way back, so the model never sees the value and you never see a");
    println!("placeholder.");
    Ok(())
}

/// Show enough to recognise a match without reprinting the secret.
///
/// A tool whose job is to keep values off a wire should not print them to a
/// terminal that is very likely being screen-shared or logged.
fn preview(matched: &str) -> String {
    let chars: Vec<char> = matched.chars().collect();
    if chars.len() <= 8 {
        return format!("{}…", chars.first().copied().unwrap_or('?'));
    }
    let head: String = chars.iter().take(4).collect();
    let tail: String = chars
        .iter()
        .rev()
        .take(2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}…{tail}  ({} chars)", chars.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preview_never_reprints_the_whole_value() {
        // This runs in a terminal that is very likely being screen-shared.
        let secret = concat!("ghp_", "abcdefghijklmnopqrstuvwxyz0123456789");
        let shown = preview(secret);
        assert!(!shown.contains(secret));
        assert!(shown.contains("ghp_"), "unrecognisable: {shown}");
        assert!(shown.contains("89"), "no tail to match against: {shown}");
    }

    #[test]
    fn a_short_value_is_not_reconstructable_from_its_preview() {
        assert_eq!(preview("abc"), "a…");
        assert_eq!(preview(""), "?…");
    }

    #[test]
    fn a_multibyte_value_does_not_panic() {
        let _ = preview("日本語のとても長い値です");
    }
}
