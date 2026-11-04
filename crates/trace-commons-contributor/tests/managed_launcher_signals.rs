//! The launcher survives the hangup a closed terminal sends: it passes it
//! on to the tool, keeps waiting, and then collects. Its own test binary,
//! because it signals this process.
#![cfg(unix)]

use trace_commons_contributor::{
    config::ConfigStore,
    managed::{
        AccountId, ToolId,
        collection_plan::{self, CollectionPlan, ProcessIdentity},
        native_collection::NativeCollection,
        profiles::NativeProfiles,
        supervisor::{TerminalSignals, wait_for_tool},
    },
};
use uuid::Uuid;

fn signal_self_after(signal: sysinfo::Signal, delay: std::time::Duration) {
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        let mut system = sysinfo::System::new();
        let me = sysinfo::Pid::from_u32(std::process::id());
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[me]), true);
        system.process(me).unwrap().kill_with(signal).unwrap();
    });
}

#[tokio::test]
async fn a_hangup_reaches_the_tool_and_the_launcher_still_collects() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let account = AccountId(Uuid::new_v4());
    let id = Uuid::new_v4();
    let root = NativeProfiles::new(&store).profile_root(account).unwrap();
    let collection = NativeCollection::snapshot(&store, ToolId::Claude, account, id, None).unwrap();

    let mut signals = TerminalSignals::new().unwrap();
    let mut child = tokio::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let pid = child.id().unwrap();
    let process = ProcessIdentity::of(pid);
    collection_plan::write(
        &store,
        &CollectionPlan {
            schema_version: 1,
            session_id: id,
            tool: ToolId::Claude,
            account_id: account,
            resume_root: None,
            cwd: dir.path().to_path_buf(),
            collect_traces: true,
            created_at: chrono::Utc::now(),
            native: Some(collection.plan()),
            opencode: None,
            process,
        },
    )
    .unwrap();
    // The tool wrote its conversation before the terminal closed.
    let transcript = root
        .join("projects/-Users-test-project")
        .join(format!("{id}.jsonl"));
    std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    std::fs::write(&transcript, format!(r#"{{"type":"user","message":{{"role":"user","content":"Please fix the test"}},"cwd":"/Users/test/project","timestamp":"2026-08-08T10:00:00Z","version":"2.0.1","sessionId":"{id}","uuid":"u1"}}
"#)).unwrap();

    signal_self_after(
        sysinfo::Signal::Hangup,
        std::time::Duration::from_millis(300),
    );
    let status = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        wait_for_tool(
            &mut child,
            pid,
            &process.label().unwrap(),
            &mut signals,
            Ok(serde_json::Value::Null),
            || Ok(serde_json::Value::Null),
        ),
    )
    .await
    .expect("the tool ends after the hangup")
    .unwrap();
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(1), "the tool got the hangup");
    }
    assert!(!process.is_running());

    // This process lived through it, and collects as the launcher does
    // after the tool exits: it takes the claim, so the daemon cannot.
    assert_eq!(
        collection_plan::claim(&store, id, &ProcessIdentity::current()),
        Ok(true)
    );
    collection.collect(&store).unwrap();
    assert!(
        store
            .dir()
            .join(trace_commons_contributor::managed::native_collection::STAGING)
            .join("claude")
            .join(id.to_string())
            .is_dir()
    );
    collection_plan::finish(&store, id);
    assert!(!collection_plan::exists(&store, id));

    // A terminate (logout) is passed on the same way.
    let mut child = tokio::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let pid = child.id().unwrap();
    let process = ProcessIdentity::of(pid);
    signal_self_after(sysinfo::Signal::Term, std::time::Duration::from_millis(300));
    let status = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        wait_for_tool(
            &mut child,
            pid,
            &process.label().unwrap(),
            &mut signals,
            Ok(serde_json::Value::Null),
            || Ok(serde_json::Value::Null),
        ),
    )
    .await
    .expect("the tool ends after the terminate")
    .unwrap();
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(status.signal(), Some(15));
}
