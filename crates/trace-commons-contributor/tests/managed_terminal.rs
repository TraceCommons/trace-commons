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
