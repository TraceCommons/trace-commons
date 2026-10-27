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
