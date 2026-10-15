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
fn only_a_bare_window_id_names_a_terminal_window() {
    use trace_commons_contributor::managed::terminal::{TerminalHandle, apple_terminal_window};
    assert_eq!(
        apple_terminal_window(b"4242\n"),
        Some(TerminalHandle::AppleTerminal { window_id: 4242 })
    );
    for output in [
        &b""[..],
        b"\n",
        b"0",
        b"-1",
        b"42 43",
        b"42\n43",
        b"window id 42",
        b"99999999999",
        b"4242; do shell script",
    ] {
        assert_eq!(apple_terminal_window(output), None, "{output:?}");
    }
    let handle = TerminalHandle::AppleTerminal { window_id: 7 };
    let json = serde_json::to_value(&handle).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"kind":"apple_terminal","window_id":7})
    );
    assert_eq!(
        serde_json::from_value::<TerminalHandle>(json).unwrap(),
        handle
    );
    assert!(
        serde_json::from_value::<TerminalHandle>(
            serde_json::json!({"kind":"apple_terminal","window_id":7,"script":"x"})
        )
        .is_err()
    );
}
