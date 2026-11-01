use std::{io::Cursor, process::Command};
use trace_commons_contributor::{
    config::ConfigStore,
    managed::{
        ManagedError,
        session_binding::{self, CodexBinding, DIRECTORY},
    },
};
use uuid::Uuid;

fn nonce(binding: &CodexBinding) -> String {
    std::fs::read_to_string(binding.directory().join("nonce")).unwrap()
}

fn input(id: Uuid, path: &std::path::Path) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"session_id":id,"transcript_path":path,"hook_event_name":"SessionStart","source":"startup","ignored_body":"must not persist"})).unwrap()
}

#[test]
fn bindings_are_private_idempotent_and_separate_between_launches() {
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().into()).unwrap();
    let first = session_binding::prepare(&store, Uuid::new_v4()).unwrap();
    let second = session_binding::prepare(&store, Uuid::new_v4()).unwrap();
    let native_id = Uuid::new_v4();
    let path = std::fs::canonicalize(home.path())
        .unwrap()
        .join("rollout.jsonl");
    std::fs::write(&path, b"{}\n").unwrap();
    session_binding::record(
        first.directory(),
        &nonce(&first),
        Cursor::new(input(native_id, &path)),
    )
    .unwrap();
    session_binding::record(
        first.directory(),
        &nonce(&first),
        Cursor::new(input(native_id, &path)),
    )
    .unwrap();
    let records = first.records().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].session_id, native_id);
    assert_eq!(records[0].transcript_path, path);
    assert!(second.records().unwrap().is_empty());
    let receipt =
        std::fs::read_to_string(first.directory().join(format!("{native_id}.json"))).unwrap();
    assert!(!receipt.contains("must not persist"));
    assert!(!receipt.contains("ignored_body"));
    assert_eq!(
        first.directory().parent().unwrap().file_name().unwrap(),
        DIRECTORY
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(first.directory())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(first.directory().join(format!("{native_id}.json")))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn recorder_rejects_wrong_nonce_invalid_ids_paths_and_oversized_documents() {
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().into()).unwrap();
    let binding = session_binding::prepare(&store, Uuid::new_v4()).unwrap();
    let file = std::fs::canonicalize(home.path())
        .unwrap()
        .join("rollout.jsonl");
    std::fs::write(&file, b"{}\n").unwrap();
    assert!(
        session_binding::record(
            binding.directory(),
            "wrong",
            Cursor::new(input(Uuid::new_v4(), &file))
        )
        .is_err()
    );
    for bytes in [b"{}".to_vec(),b"not json".to_vec(),vec![b'x';65537],serde_json::to_vec(&serde_json::json!({"session_id":"../../escape","transcript_path":file,"hook_event_name":"SessionStart"})).unwrap(),serde_json::to_vec(&serde_json::json!({"session_id":Uuid::new_v4(),"transcript_path":"relative.jsonl","hook_event_name":"SessionStart"})).unwrap()] {
        assert!(session_binding::record(binding.directory(),&nonce(&binding),Cursor::new(bytes)).is_err());
    }
    assert!(binding.records().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn configuration_approves_only_the_exact_collector_and_keeps_native_profile() {
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().into()).unwrap();
    let binding = session_binding::prepare(&store, Uuid::new_v4()).unwrap();
    let mut command = Command::new("codex");
    command.env("CODEX_HOME", home.path());
    binding.configure(&mut command).unwrap();
    let args = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(!args.iter().any(|arg| arg == "--no-daemon"));
    assert!(
        args.iter()
            .any(|arg| arg.starts_with("hooks.SessionStart="))
    );
    let state = args
        .iter()
        .find(|arg| arg.starts_with("hooks.state="))
        .unwrap();
    let parsed: toml::Table = state.parse().unwrap();
    let states = parsed["hooks"]["state"].as_table().unwrap();
    assert_eq!(states.len(), 1);
    assert!(
        states.values().next().unwrap()["trusted_hash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert!(!args.iter().any(|arg| arg.contains("dangerously-bypass")));
    assert!(!args.iter().any(|arg| arg == "--enable"));
    assert!(
        command
            .get_envs()
            .any(|(key, value)| key == "CODEX_HOME" && value == Some(home.path().as_os_str()))
    );
}

#[test]
fn custom_cli_hook_configuration_is_not_overwritten() {
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().into()).unwrap();
    let binding = session_binding::prepare(&store, Uuid::new_v4()).unwrap();
    let mut terminated = Command::new("codex");
    terminated.args(["exec", "--", "prompt"]);
    assert_eq!(
        binding.configure(&mut terminated),
        Err(ManagedError::InvalidRequest)
    );
    assert_eq!(terminated.get_args().count(), 3);
    for config in [
        "hooks.SessionStart=[]",
        "hooks.state={}",
        "\"hooks\".SessionStart=[]",
    ] {
        let mut command = Command::new("codex");
        command.args(["-c", config]);
        assert_eq!(
            binding.configure(&mut command),
            Err(ManagedError::InvalidRequest)
        );
        assert_eq!(command.get_args().count(), 2);
    }
}

#[cfg(unix)]
#[test]
fn symlinked_receipt_and_transcript_paths_are_refused() {
    use std::os::unix::fs::symlink;
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().into()).unwrap();
    let binding = session_binding::prepare(&store, Uuid::new_v4()).unwrap();
    let root = std::fs::canonicalize(home.path()).unwrap();
    let file = root.join("original.jsonl");
    std::fs::write(&file, b"{}\n").unwrap();
    let link = root.join("linked.jsonl");
    symlink(&file, &link).unwrap();
    assert!(
        session_binding::record(
            binding.directory(),
            &nonce(&binding),
            Cursor::new(input(Uuid::new_v4(), &link))
        )
        .is_err()
    );
    let id = Uuid::new_v4();
    symlink(&file, binding.directory().join(format!("{id}.json"))).unwrap();
    assert!(
        session_binding::record(
            binding.directory(),
            &nonce(&binding),
            Cursor::new(input(id, &file))
        )
        .is_err()
    );
    assert!(binding.records().is_err());
}

#[test]
fn wipe_removes_binding_metadata_but_preserves_native_transcript() {
    let home = tempfile::tempdir().unwrap();
    let native = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().into()).unwrap();
    let binding = session_binding::prepare(&store, Uuid::new_v4()).unwrap();
    let transcript = std::fs::canonicalize(native.path())
        .unwrap()
        .join("rollout.jsonl");
    std::fs::write(&transcript, b"native source\n").unwrap();
    session_binding::record(
        binding.directory(),
        &nonce(&binding),
        Cursor::new(input(Uuid::new_v4(), &transcript)),
    )
    .unwrap();
    store.wipe().unwrap();
    assert!(!home.path().join(DIRECTORY).exists());
    assert_eq!(std::fs::read(&transcript).unwrap(), b"native source\n");
}

#[cfg(unix)]
#[test]
fn installed_codex_exec_help_accepts_binding_configuration() {
    // Native CLI smoke check when Codex is available; help performs no model call.
    let Ok(version) = Command::new("codex").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().into()).unwrap();
    let binding = session_binding::prepare(&store, Uuid::new_v4()).unwrap();
    for subcommand in ["exec", "resume", "fork"] {
        let mut command = Command::new("codex");
        command.args([subcommand, "--help"]);
        binding.configure(&mut command).unwrap();
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{subcommand}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// A resume reads the conversation an earlier launch recorded, without
/// preparing its binding again (that is a conflict). Only exactly one
/// record whose transcript is still there names a conversation.
#[test]
fn a_resume_reads_the_recorded_conversation_and_never_prepares_it_again() {
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().into()).unwrap();
    let id = Uuid::new_v4();
    assert_eq!(session_binding::read(&store, id), None, "no binding");
    let binding = session_binding::prepare(&store, id).unwrap();
    assert_eq!(session_binding::read(&store, id), None, "no record");

    let native_id = Uuid::new_v4();
    let path = std::fs::canonicalize(home.path())
        .unwrap()
        .join("rollout.jsonl");
    std::fs::write(&path, b"{}\n").unwrap();
    session_binding::record(
        binding.directory(),
        &nonce(&binding),
        Cursor::new(input(native_id, &path)),
    )
    .unwrap();
    let read = session_binding::read(&store, id).unwrap();
    assert_eq!(read.session_id, native_id);
    assert_eq!(read.transcript_path, path);
    assert!(matches!(
        session_binding::prepare(&store, id),
        Err(ManagedError::Conflict)
    ));
    assert_eq!(
        session_binding::read(&store, id),
        Some(read),
        "reading changes nothing"
    );

    // A second conversation in one launch is ambiguous.
    let other = std::fs::canonicalize(home.path())
        .unwrap()
        .join("other.jsonl");
    std::fs::write(&other, b"{}\n").unwrap();
    session_binding::record(
        binding.directory(),
        &nonce(&binding),
        Cursor::new(input(Uuid::new_v4(), &other)),
    )
    .unwrap();
    assert_eq!(session_binding::read(&store, id), None);
    std::fs::remove_file(&other).unwrap();
    assert_eq!(
        session_binding::read(&store, id),
        None,
        "a record whose transcript is gone is invalid"
    );
}

/// The daemon collects a row whose launcher is gone by opening that row's
/// binding again: it reads the same records, under the same checks.
#[test]
fn a_binding_reopened_for_collection_reads_the_same_records() {
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().into()).unwrap();
    let row = Uuid::new_v4();
    let binding = session_binding::prepare(&store, row).unwrap();
    let native_id = Uuid::new_v4();
    let path = std::fs::canonicalize(home.path())
        .unwrap()
        .join("rollout.jsonl");
    std::fs::write(&path, b"{}\n").unwrap();
    session_binding::record(
        binding.directory(),
        &nonce(&binding),
        Cursor::new(input(native_id, &path)),
    )
    .unwrap();
    let reopened = session_binding::reopen(&store, row).unwrap();
    assert_eq!(reopened.directory(), binding.directory());
    assert_eq!(reopened.records().unwrap(), binding.records().unwrap());
    assert!(session_binding::reopen(&store, Uuid::new_v4()).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(binding.directory(), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        assert!(
            session_binding::reopen(&store, row).is_err(),
            "a directory others can read is not the launch's"
        );
    }
}
