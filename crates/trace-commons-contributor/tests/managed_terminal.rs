#![cfg(unix)]

use std::path::Path;
use trace_commons_contributor::managed::terminal::shell_command;

#[test]
fn terminal_handoff_quotes_paths_and_never_accepts_shell_syntax_as_arguments() {
    let command = shell_command(
        Path::new("/tmp/a 'quote'/near-ai"),
        Path::new("/tmp/$(touch BAD)"),
        uuid::Uuid::nil(),
        "abcdef",
    )
    .unwrap();
    assert!(command.contains("'\\''"));
    assert!(command.contains("'/tmp/$(touch BAD)'"));
    assert!(command.ends_with("--ticket 'abcdef'"));
    assert!(
        shell_command(
            Path::new("near-ai"),
            Path::new("/tmp"),
            uuid::Uuid::nil(),
            "abcdef"
        )
        .is_err()
    );
    assert!(
        shell_command(
            Path::new("/tmp/near-ai"),
            Path::new("/tmp"),
            uuid::Uuid::nil(),
            "a; command"
        )
        .is_err()
    );
}

#[test]
fn only_a_bare_tty_path_names_a_terminal_tab() {
    use trace_commons_contributor::managed::terminal::{TerminalHandle, apple_terminal_tab};
    assert_eq!(
        apple_terminal_tab(b"/dev/ttys003\n"),
        Some(TerminalHandle::AppleTerminal {
            tty: "/dev/ttys003".into()
        })
    );
    for output in [
        &b""[..],
        b"\n",
        b"/dev/tty",
        b"ttys003",
        b"/dev/ttys003 /dev/ttys004",
        b"/dev/ttys003\n/dev/ttys004",
        b"/dev/ttys003\"; do shell script",
        b"/dev/../etc/passwd",
        b"/dev/ttys0/../x",
        b"/dev/ttys00000000000000000000",
        b"4242",
    ] {
        assert_eq!(apple_terminal_tab(output), None, "{output:?}");
    }
    let handle = TerminalHandle::AppleTerminal {
        tty: "/dev/ttys007".into(),
    };
    let json = serde_json::to_value(&handle).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"kind":"apple_terminal","tty":"/dev/ttys007"})
    );
    assert_eq!(
        serde_json::from_value::<TerminalHandle>(json).unwrap(),
        handle
    );
    assert!(
        serde_json::from_value::<TerminalHandle>(
            serde_json::json!({"kind":"apple_terminal","tty":"/dev/ttys007","script":"x"})
        )
        .is_err()
    );
    // A handle read from disk is held to the same shape as one from a launch.
    assert!(
        serde_json::from_value::<TerminalHandle>(
            serde_json::json!({"kind":"apple_terminal","tty":"x\" & (do shell script \"y\") & \""})
        )
        .is_err()
    );
}

/// Run the launch command the way each macOS terminal does, with the shell
/// started at `start_soft` open files, and return what the stand-in helper
/// printed. `argv` is the terminal's own invocation, `COMMAND` standing for
/// the launch string as one argument. The helper is a stand-in that prints
/// its soft limit and arguments.
fn run_launch(argv: &[&str], start_soft: u32) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let helper = dir.path().join("near-ai");
    std::fs::write(
        &helper,
        "#!/bin/sh\nprintf 'HELPER RAN soft=%s args=%s\\n' \"$(ulimit -Sn)\" \"$*\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let command = shell_command(&helper, dir.path(), uuid::Uuid::nil(), "abcdef").unwrap();
    let out = std::process::Command::new("/bin/sh")
        // A hard limit below the start value cannot be set up: skip then.
        .args([
            "-c",
            "ulimit -Sn \"$0\" 2>/dev/null || exit 99; exec \"$@\"",
        ])
        .arg(start_soft.to_string())
        .args(argv.iter().map(|a| a.replace("COMMAND", &command)))
        .output()
        .unwrap();
    if out.status.code() == Some(99) {
        return None;
    }
    assert!(out.status.success(), "{out:?}");
    Some(String::from_utf8(out.stdout).unwrap())
}

fn soft_limit_seen(output: &str) -> u32 {
    let soft = output
        .split("soft=")
        .nth(1)
        .unwrap()
        .split(' ')
        .next()
        .unwrap();
    soft.parse().unwrap()
}

/// Terminal.app types the line into a shell; Ghostty runs it as
/// `bash -c "exec -l <command>"`, so a `ulimit ...;` prefix would have
/// replaced the shell with /usr/bin/ulimit and dropped the helper.
const LAUNCH_STYLES: [&[&str]; 2] = [
    &["/bin/bash", "--noprofile", "--norc", "-c", "COMMAND"],
    &[
        "/bin/bash",
        "--noprofile",
        "--norc",
        "-c",
        "exec -l COMMAND",
    ],
];

#[test]
fn every_terminal_style_runs_the_helper_with_a_raised_soft_limit() {
    for argv in LAUNCH_STYLES {
        let Some(out) = run_launch(argv, 256) else {
            continue;
        };
        assert!(out.starts_with("HELPER RAN"), "{argv:?}: {out}");
        assert!(out.contains("redeem-launch --session-id"), "{out}");
        assert!(soft_limit_seen(&out) >= 10240, "{argv:?}: {out}");
    }
}

#[test]
fn a_higher_soft_limit_is_never_lowered() {
    for argv in LAUNCH_STYLES {
        let Some(out) = run_launch(argv, 20000) else {
            continue;
        };
        assert_eq!(soft_limit_seen(&out), 20000, "{argv:?}: {out}");
    }
}

#[test]
fn the_launch_is_one_simple_command_that_needs_no_shell_to_split() {
    // iTerm2's default profile splits the string and execs it with no shell.
    let command = shell_command(
        Path::new("/tmp/near-ai"),
        Path::new("/tmp"),
        uuid::Uuid::nil(),
        "abcdef",
    )
    .unwrap();
    assert!(command.starts_with("/bin/sh -c '"), "{command}");
    assert!(!command.starts_with("ulimit"), "{command}");
    assert!(command.contains("' '/tmp/near-ai' --config-dir '/tmp' "));
}
