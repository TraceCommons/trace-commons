//! `--version` names the build channel (staging design 4.3).
//!
//! The macOS bundle scripts read the word after "channel " at the end of the
//! bundled `near-ai --version` line and refuse to sign a bundle whose
//! Info.plist `TCChannel` disagrees with it. This holds that shape for both
//! binaries the crate ships.

use std::process::Command;

#[cfg(not(feature = "near-ai-staging"))]
const EXPECTED: &str = "production";
#[cfg(feature = "near-ai-staging")]
const EXPECTED: &str = "staging";

fn version_line(binary: &str) -> String {
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .expect("the binary runs");
    assert!(output.status.success(), "--version exits 0");
    String::from_utf8(output.stdout)
        .expect("utf-8")
        .trim_end()
        .to_owned()
}

#[test]
fn the_bundled_cli_names_its_channel_last() {
    let line = version_line(env!("CARGO_BIN_EXE_near-ai"));
    assert!(
        line.ends_with(&format!(" channel {EXPECTED}")),
        "near-ai --version must end with its channel: {line}"
    );
    assert_eq!(line.matches(" channel ").count(), 1, "{line}");
}

#[test]
fn the_contributor_cli_names_its_channel_last() {
    let line = version_line(env!("CARGO_BIN_EXE_trace-commons-contributor"));
    assert!(
        line.ends_with(&format!(" channel {EXPECTED}")),
        "trace-commons-contributor --version must end with its channel: {line}"
    );
    // The build identity stays first: callers match the binary name.
    assert!(line.starts_with("trace-commons-contributor "), "{line}");
    assert!(line.contains("(commit "), "{line}");
}
