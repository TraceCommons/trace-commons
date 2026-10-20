use ironwire_ledger::token_spool::*;
#[test]
fn releasing_one_destination_never_deletes_another_or_a_late_turn() {
    let home = tempfile::tempdir().unwrap();
    let spool = TokenSpool::open(home.path(), 1024, 100).unwrap();
    let first = spool
        .record(
            "session",
            1,
            "openai.chat",
            true,
            (b"request", b"response"),
            1,
        )
        .unwrap();
    let a = spool
        .acquire(
            "session",
            std::slice::from_ref(&first.capture_id),
            "commons-a",
            2,
            90,
        )
        .unwrap();
    let b = spool
        .acquire(
            "session",
            std::slice::from_ref(&first.capture_id),
            "commons-b",
            2,
            90,
        )
        .unwrap();
    let late = spool
        .record("session", 2, "openai.chat", true, (b"late", b"turn"), 3)
        .unwrap();
    spool
        .release(&a.lease_id, &a.owner, &a.snapshot_digest, 4)
        .unwrap();
    assert!(
        spool
            .read(&b.lease_id, &b.owner, &first.capture_id, 4)
            .is_ok()
    );
    assert!(
        spool
            .release(&b.lease_id, "wrong-owner", &b.snapshot_digest, 4)
            .is_err()
    );
    spool
        .release(&b.lease_id, &b.owner, &b.snapshot_digest, 5)
        .unwrap();
    let remaining = spool.list("session", 5).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].capture_id, late.capture_id);
    spool
        .release(&b.lease_id, &b.owner, &b.snapshot_digest, 5)
        .unwrap();
}
#[test]
fn a_lease_survives_restart_and_pruning_and_checks_exact_bytes() {
    let home = tempfile::tempdir().unwrap();
    let (lease, id) = {
        let spool = TokenSpool::open(home.path(), 1024, 10).unwrap();
        let capture = spool
            .record(
                "session",
                1,
                "openai.chat",
                false,
                (b"{  \"x\":1}", b"original"),
                1,
            )
            .unwrap();
        let lease = spool
            .acquire(
                "session",
                std::slice::from_ref(&capture.capture_id),
                "owner",
                2,
                100,
            )
            .unwrap();
        (lease, capture.capture_id)
    };
    let spool = TokenSpool::open(home.path(), 1024, 10).unwrap();
    spool.prune(20).unwrap();
    assert_eq!(
        spool
            .read(&lease.lease_id, &lease.owner, &id, 20)
            .unwrap()
            .0,
        b"{  \"x\":1}"
    );
    std::fs::write(home.path().join(format!("{id}.res")), b"tampered").unwrap();
    assert!(spool.read(&lease.lease_id, &lease.owner, &id, 20).is_err());
    spool.prune(103).unwrap();
    assert!(spool.read(&lease.lease_id, &lease.owner, &id, 103).is_err());
}
#[test]
fn capacity_never_evicts_pending_captures_and_ids_cannot_select_paths() {
    let home = tempfile::tempdir().unwrap();
    let spool = TokenSpool::open(home.path(), 10, 100).unwrap();
    let capture = spool
        .record("session", 1, "openai.chat", false, (b"12345", b"67890"), 1)
        .unwrap();
    assert!(matches!(
        spool.record("session", 2, "openai.chat", false, (b"x", b"x"), 2),
        Err(SpoolError::Capacity)
    ));
    assert_eq!(spool.list("session", 2).unwrap().len(), 1);
    assert!(
        spool
            .acquire("other-session", &[capture.capture_id], "owner", 2, 10)
            .is_err()
    );
    assert!(
        spool
            .acquire("session", &["../private".into()], "owner", 2, 10)
            .is_err()
    );
}
#[test]
fn garbage_collection_does_not_follow_payload_symlinks() {
    #[cfg(unix)]
    {
        let home = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(outside.path(), b"keep").unwrap();
        let spool = TokenSpool::open(home.path(), 1024, 10).unwrap();
        let capture = spool
            .record("session", 1, "openai.chat", false, (b"a", b"b"), 1)
            .unwrap();
        let path = home.path().join(format!("{}.res", capture.capture_id));
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(outside.path(), &path).unwrap();
        spool.prune(20).unwrap();
        assert_eq!(std::fs::read(outside.path()).unwrap(), b"keep");
    }
}

#[test]
fn renewal_preserves_snapshot_and_has_a_hard_lifetime() {
    use ironwire_ledger::token_spool::MAX_LIFETIME_SECONDS;
    let home = tempfile::tempdir().unwrap();
    let spool = TokenSpool::open(home.path(), 1024, 10).unwrap();
    let capture = spool
        .record(
            "session",
            1,
            "openai.chat",
            false,
            (b"request", b"response"),
            1,
        )
        .unwrap();
    let lease = spool
        .acquire(
            "session",
            std::slice::from_ref(&capture.capture_id),
            "owner",
            2,
            20,
        )
        .unwrap();
    assert!(
        spool
            .renew(&lease.lease_id, "other", &lease.snapshot_digest, 3, 30)
            .is_err()
    );
    let end = spool
        .renew(
            &lease.lease_id,
            &lease.owner,
            &lease.snapshot_digest,
            3,
            MAX_LIFETIME_SECONDS,
        )
        .unwrap();
    assert_eq!(end, 1 + MAX_LIFETIME_SECONDS);
    spool.prune(100).unwrap();
    assert!(
        spool
            .read(&lease.lease_id, &lease.owner, &capture.capture_id, 100)
            .is_ok()
    );
    spool.prune(end).unwrap();
    assert!(
        spool
            .renew(
                &lease.lease_id,
                &lease.owner,
                &lease.snapshot_digest,
                end,
                1
            )
            .is_err()
    );
    spool
        .release(
            &lease.lease_id,
            &lease.owner,
            &lease.snapshot_digest,
            end + 1,
        )
        .unwrap();
}

#[test]
fn final_exchange_remains_discoverable_after_128_retained_turns() {
    use ironwire_ledger::token_spool::TokenSpool;
    let dir = tempfile::tempdir().unwrap();
    let spool = TokenSpool::open(&dir.path().join("spool"), 1024 * 1024, 86400).unwrap();
    let mut last = None;
    for i in 1..=150 {
        let body = format!("turn-{i}");
        last = Some(
            spool
                .record(
                    "session",
                    i,
                    "openai.chat",
                    false,
                    (body.as_bytes(), b"reply"),
                    i,
                )
                .unwrap(),
        );
    }
    let last = last.unwrap();
    assert_eq!(spool.list("session", 200).unwrap().len(), 128);
    let found = spool
        .find("session", &last.request_digest, &last.response_digest, 200)
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].capture_id, last.capture_id);
    assert!(
        spool
            .find("other", &last.request_digest, &last.response_digest, 200)
            .unwrap()
            .is_empty()
    );
    spool
        .record(
            "session",
            151,
            "openai.chat",
            false,
            (b"turn-150", b"reply"),
            151,
        )
        .unwrap();
    assert_eq!(
        spool
            .find("session", &last.request_digest, &last.response_digest, 200)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn a_session_cannot_fill_the_global_spool() {
    use ironwire_ledger::token_spool::TokenSpool;
    let dir = tempfile::tempdir().unwrap();
    let spool = TokenSpool::open(&dir.path().join("spool"), 128 * 1024 * 1024, 86400).unwrap();
    let body = vec![b'x'; 32 * 1024 * 1024];
    spool
        .record("busy", 1, "openai.chat", false, (&body, &body), 1)
        .unwrap();
    assert!(
        spool
            .record("busy", 2, "openai.chat", false, (b"a", b"b"), 2)
            .is_err()
    );
    assert!(
        spool
            .record("other", 3, "openai.chat", false, (b"a", b"b"), 3)
            .is_ok()
    );
}

#[cfg(windows)]
#[test]
fn raw_spool_has_a_protected_user_only_acl() {
    use ironwire_ledger::token_spool::TokenSpool;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("spool");
    let spool = TokenSpool::open(&root, 1024 * 1024, 86400).unwrap();
    spool
        .record(
            "session",
            1,
            "openai.chat",
            false,
            (b"private-request", b"private-response"),
            1,
        )
        .unwrap();
    let script = r#"
$ErrorActionPreference = 'Stop'
$sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$paths = @((Get-Item -LiteralPath $env:IRONWIRE_TEST_ROOT)) + @(Get-ChildItem -LiteralPath $env:IRONWIRE_TEST_ROOT)
foreach ($item in $paths) {
  $acl = $item.GetAccessControl()
  foreach ($rule in $acl.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier])) {
    if ($rule.IdentityReference.Value -ne $sid) { exit 2 }
  }
}
if (!(Get-Item -LiteralPath $env:IRONWIRE_TEST_ROOT -Force).GetAccessControl().AreAccessRulesProtected) { exit 3 }
"#;
    assert!(
        std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .env("IRONWIRE_TEST_ROOT", &root)
            .status()
            .unwrap()
            .success()
    );
}

#[cfg(windows)]
#[test]
fn raw_spool_refuses_directory_junctions() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    let link = dir.path().join("link");
    std::fs::create_dir(&target).unwrap();
    let status = std::process::Command::new("cmd.exe")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&target)
        .output()
        .unwrap();
    assert!(status.status.success());
    assert!(TokenSpool::open(&link, 1024, 100).is_err());
    assert!(TokenSpool::open(&link.join("nested"), 1024, 100).is_err());
    assert!(!target.join("spool.sqlite").exists());
    std::fs::remove_dir(link).unwrap();
}

#[cfg(windows)]
#[test]
fn exclusive_file_handles_fail_closed_without_replacing_the_database() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("spool");
    let spool = TokenSpool::open(&root, 1024, 100).unwrap();
    let id = spool.store_id().unwrap();
    drop(spool);
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(root.join("spool.sqlite"))
        .unwrap();
    assert!(TokenSpool::open(&root, 1024, 100).is_err());
    drop(held);
    assert_eq!(
        TokenSpool::open(&root, 1024, 100)
            .unwrap()
            .store_id()
            .unwrap(),
        id
    );
}

#[cfg(windows)]
#[test]
fn canonical_windows_paths_support_the_embedded_host() {
    let dir = tempfile::tempdir().unwrap();
    let canonical = std::fs::canonicalize(dir.path()).unwrap();
    let spool = TokenSpool::open(&canonical.join("spool"), 1024, 100).unwrap();
    assert!(spool.store_id().is_ok());
}
