//! A host that serves the status line from its own executable says so.
//!
//! `StatusLine::Offer` writes `current_exe() statusline`, which is right only
//! for a binary with that subcommand. `StatusLine::Command` lets a host name
//! the command that actually serves the line, under the same slot rules as
//! IronWire's own: fill only an empty slot, follow our own line when it moves,
//! and take out only what IronWire put there.
//!
//! In a child process because the default login's home is process-global.
//! A custom CLAUDE_CONFIG_DIR must not redirect an edit of the plain facade.

use ironwire_agents::tools::{
    ConnectOptions, StatusLine, commit, plan_connect_with, plan_disconnect,
};
use ironwire_catalog::schema::Catalog;

const CHILD: &str = "IRONWIRE_HOST_STATUS_LINE_TEST";
const HOST: &str = "/opt/host/bin/host-cli ironwire-statusline --session 7";

fn read(dir: &std::path::Path) -> serde_json::Value {
    let text = std::fs::read_to_string(dir.join("settings.json")).unwrap_or_default();
    if text.trim().is_empty() {
        return serde_json::json!({});
    }
    serde_json::from_str(&text).unwrap()
}

fn host() -> ConnectOptions {
    ConnectOptions::default().with_status_line(StatusLine::Command(HOST.to_string()))
}

fn scenario(dir: &std::path::Path) {
    let catalog = Catalog::default();
    let settings = dir.join("settings.json");

    // An empty slot takes the host's command verbatim, marked as ours.
    let planned = plan_connect_with("claude", 8463, &catalog, host()).expect("plans");
    commit(&planned).expect("commits");
    let wired = read(dir);
    assert_eq!(wired["statusLine"]["command"], HOST);
    assert_eq!(wired["statusLine"]["installedBy"], "ironwire");
    assert_eq!(
        wired["env"]["ANTHROPIC_BASE_URL"],
        "http://127.0.0.1:8463/anthropic"
    );

    // Again is nothing.
    let again = plan_connect_with("claude", 8463, &catalog, host()).expect("plans");
    assert!(again.changes.is_empty(), "{:?}", again.changes);

    // Disconnect takes out exactly what connect put in.
    let planned = plan_disconnect("claude", &catalog).expect("plans");
    commit(&planned).expect("commits");
    let unwired = read(dir);
    assert!(unwired.get("statusLine").is_none(), "{unwired}");
    assert!(unwired.get("env").is_none(), "{unwired}");

    // A line IronWire installed earlier, naming an executable that cannot
    // serve it, is ours and follows the host to the command that can.
    std::fs::write(
        &settings,
        r#"{"statusLine":{"type":"command","command":"/Applications/Host.app/Contents/MacOS/Host statusline","installedBy":"ironwire"}}"#,
    )
    .unwrap();
    let planned = plan_connect_with("claude", 8463, &catalog, host()).expect("plans");
    commit(&planned).expect("commits");
    assert_eq!(read(dir)["statusLine"]["command"], HOST);

    // A line the user wrote is theirs, and is reported rather than replaced.
    let theirs = r#"{"statusLine":{"type":"command","command":"~/bin/my-prompt.sh"}}"#;
    std::fs::write(&settings, theirs).unwrap();
    let planned = plan_connect_with("claude", 8463, &catalog, host()).expect("plans");
    assert!(
        planned
            .occupied
            .iter()
            .any(|(slot, current)| slot == "statusLine" && current == "~/bin/my-prompt.sh"),
        "{:?}",
        planned.occupied
    );
    commit(&planned).expect("commits");
    assert_eq!(read(dir)["statusLine"]["command"], "~/bin/my-prompt.sh");
    let planned = plan_disconnect("claude", &catalog).expect("plans");
    commit(&planned).expect("commits");
    assert_eq!(read(dir)["statusLine"]["command"], "~/bin/my-prompt.sh");
}

#[test]
fn a_host_installs_its_own_status_line_command_under_the_same_rules() {
    if let Some(dir) = std::env::var_os(CHILD) {
        let home = std::path::Path::new(&dir);
        let default = home.join(".claude");
        std::fs::create_dir_all(&default).unwrap();
        scenario(&default);
        assert!(!home.join("other-login/settings.json").exists());
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "a_host_installs_its_own_status_line_command_under_the_same_rules",
            "--nocapture",
        ])
        .env_clear()
        .env(CHILD, dir.path())
        .env("CLAUDE_CONFIG_DIR", dir.path().join("other-login"))
        .env("HOME", dir.path())
        .env("USERPROFILE", dir.path())
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // The child really ran the scenario, rather than the filter matching
    // nothing and passing vacuously.
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}
