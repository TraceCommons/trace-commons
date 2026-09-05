use super::*;
use serde_json::{Value, json};

fn expected() -> ArtifactExpectation<'static> {
    ArtifactExpectation {
        source_revision: "1111111111111111111111111111111111111111",
        compatibility_id: "tc-compute-v1",
        signing_identifier: "org.example.test-worker",
        signing_team: "TESTTEAM01",
        host_target: "aarch64-apple-darwin",
        host_macos: [15, 0, 0],
    }
}

// Minimal structural header, not signed code or proof of backend readiness.
fn executable() -> Vec<u8> {
    [
        0xfeedfacfu32,
        0x0100000c,
        0,
        2,
        1,
        24,
        0,
        0,
        0x32,
        24,
        1,
        15 << 16,
        15 << 16,
        0,
    ]
    .into_iter()
    .flat_map(u32::to_le_bytes)
    .collect()
}
fn manifest() -> Value {
    json!({
        "schema_version": 1,
        "source_revision": expected().source_revision,
        "target": "aarch64-apple-darwin", "backend": "mlx",
        "minimum_macos": [15, 0, 0], "ipc_version": 0,
        "compatibility_id": expected().compatibility_id,
        "signing_identifier": expected().signing_identifier,
        "signing_team": expected().signing_team,
        "worker": {"size_bytes": executable().len(), "sha256": hex::encode(Sha256::digest(executable()))},
        "assets": [{"relative_path": "mlx.metallib", "size_bytes": 5, "sha256": hex::encode(Sha256::digest(b"asset"))}]
    })
}
fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("Contents/Helpers")).unwrap();
    std::fs::create_dir_all(root.path().join("Contents/Resources/Compute/assets")).unwrap();
    std::fs::write(root.path().join(WORKER_PATH), executable()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            root.path().join(WORKER_PATH),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    std::fs::write(
        root.path()
            .join("Contents/Resources/Compute/assets/mlx.metallib"),
        b"asset",
    )
    .unwrap();
    write_manifest(root.path(), &manifest());
    root
}
fn write_manifest(root: &Path, value: &Value) {
    std::fs::write(root.join(MANIFEST_PATH), serde_json::to_vec(value).unwrap()).unwrap();
}
fn parse(value: &Value) -> Result<Manifest, ArtifactError> {
    Manifest::parse(&serde_json::to_vec(value).unwrap(), &expected())
}

#[test]
fn valid_inventory_is_read_only_and_not_a_launch_capability() {
    let root = fixture();
    assert_eq!(
        check_integrity(root.path(), &expected()).unwrap(),
        IntegrityChecked {
            asset_count: 1,
            checked_bytes: executable().len() as u64 + 5
        }
    );
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    assert!(!root.path().join("compute").exists());
}

#[test]
fn schema_and_bounds_fail_closed() {
    for (key, value) in [
        ("schema_version", json!(2)),
        ("assets", json!([])),
        ("unknown", json!(true)),
        ("minimum_macos", json!([15, 256, 0])),
        ("backend", json!(null)),
        ("source_revision", json!("short")),
        ("signing_team", json!("bad")),
        (
            "assets",
            json!(vec![manifest()["assets"][0].clone(); MAX_ENTRIES + 1]),
        ),
    ] {
        let mut m = manifest();
        m[key] = value;
        assert!(matches!(parse(&m), Err(ArtifactError::Manifest)), "{key}");
    }
    for field in ["sha256", "size_bytes"] {
        let mut m = manifest();
        m["worker"].as_object_mut().unwrap().remove(field);
        assert!(matches!(parse(&m), Err(ArtifactError::Manifest)));
    }
    for bytes in [
        b"{".to_vec(),
        vec![b' '; MAX_MANIFEST as usize + 1],
        b"{\"schema_version\":1,\"schema_version\":1}".to_vec(),
    ] {
        assert!(matches!(
            Manifest::parse(&bytes, &expected()),
            Err(ArtifactError::Manifest)
        ));
    }
    let mut m = manifest();
    m["worker"]["size_bytes"] = json!(MAX_FILE + 1);
    assert!(matches!(parse(&m), Err(ArtifactError::Manifest)));
    let mut m = manifest();
    m["worker"]["sha256"] = json!("z".repeat(64));
    assert!(matches!(parse(&m), Err(ArtifactError::Manifest)));
    let mut m = manifest();
    m["assets"][0]["unknown"] = json!(0);
    assert!(matches!(parse(&m), Err(ArtifactError::Manifest)));
    let mut m = manifest();
    m["assets"] = json!((0..3).map(|i| json!({"relative_path":format!("asset-{i}"), "size_bytes":MAX_FILE, "sha256":"a".repeat(64)})).collect::<Vec<_>>());
    assert!(matches!(parse(&m), Err(ArtifactError::Manifest)));
}

#[test]
fn release_and_host_must_match_independent_expectation() {
    for (key, value) in [
        ("target", json!("x86_64-apple-darwin")),
        ("backend", json!("cpu")),
        ("ipc_version", json!(1)),
        ("compatibility_id", json!("other")),
        ("source_revision", json!("2".repeat(40))),
        ("signing_identifier", json!("org.other.worker")),
        ("signing_team", json!("OTHERTEAM1")),
        ("minimum_macos", json!([16, 0, 0])),
    ] {
        let mut m = manifest();
        m[key] = value;
        assert!(
            matches!(parse(&m), Err(ArtifactError::Incompatible)),
            "{key}"
        );
    }
    let root = fixture();
    let mut e = expected();
    e.host_target = "x86_64-apple-darwin";
    assert_eq!(
        check_integrity(root.path(), &e),
        Err(ArtifactError::Incompatible)
    );
}

#[test]
fn traversal_aliases_and_case_collisions_are_refused() {
    for path in [
        "../outside",
        "/tmp/asset",
        "a/../b",
        "a//b",
        "./a",
        "a/",
        "a\\b",
        "C:asset",
        "",
        "é",
        "a\0b",
    ] {
        let mut m = manifest();
        m["assets"][0]["relative_path"] = json!(path);
        assert!(
            matches!(parse(&m), Err(ArtifactError::Manifest)),
            "{path:?}"
        );
    }
    for name in ["mlx.metallib", "MLX.metallib"] {
        let mut m = manifest();
        let mut duplicate = m["assets"][0].clone();
        duplicate["relative_path"] = json!(name);
        m["assets"].as_array_mut().unwrap().push(duplicate);
        assert!(matches!(parse(&m), Err(ArtifactError::Manifest)));
    }
}

#[test]
fn missing_modified_and_non_regular_files_are_refused() {
    for path in [
        WORKER_PATH,
        "Contents/Resources/Compute/assets/mlx.metallib",
    ] {
        let root = fixture();
        let file = root.path().join(path);
        let mut bytes = std::fs::read(&file).unwrap();
        bytes[0] ^= 1;
        std::fs::write(&file, bytes).unwrap();
        assert_eq!(
            check_integrity(root.path(), &expected()),
            Err(ArtifactError::Integrity)
        );
        std::fs::remove_file(&file).unwrap();
        assert_eq!(
            check_integrity(root.path(), &expected()),
            Err(ArtifactError::Read)
        );
        std::fs::create_dir(&file).unwrap();
        assert_eq!(
            check_integrity(root.path(), &expected()),
            Err(ArtifactError::Path)
        );
    }
    let root = fixture();
    std::fs::write(
        root.path().join(MANIFEST_PATH),
        vec![b' '; MAX_MANIFEST as usize + 1],
    )
    .unwrap();
    assert_eq!(
        check_integrity(root.path(), &expected()),
        Err(ArtifactError::Manifest)
    );
}

#[test]
fn signed_byte_change_requires_new_pin() {
    let root = fixture();
    let mut modified = executable();
    modified.extend_from_slice(b"synthetic-signature-change");
    std::fs::write(root.path().join(WORKER_PATH), &modified).unwrap();
    assert_eq!(
        check_integrity(root.path(), &expected()),
        Err(ArtifactError::Integrity)
    );
    let mut m = manifest();
    m["worker"]["size_bytes"] = json!(modified.len());
    m["worker"]["sha256"] = json!(hex::encode(Sha256::digest(&modified)));
    write_manifest(root.path(), &m);
    assert!(check_integrity(root.path(), &expected()).is_ok());
}

#[test]
fn actual_header_must_match_manifest_even_with_matching_hash() {
    for (offset, word) in [
        (0, 0xcafebabe),
        (4, 0x01000007),
        (12, 6),
        (20, u32::MAX),
        (32, 0x24),
        (36, 0),
        (40, 2),
        (44, 16 << 16),
        (52, 1),
    ] {
        let root = fixture();
        let mut bytes = executable();
        bytes[offset..offset + 4].copy_from_slice(&u32::to_le_bytes(word));
        std::fs::write(root.path().join(WORKER_PATH), &bytes).unwrap();
        let mut m = manifest();
        m["worker"]["sha256"] = json!(hex::encode(Sha256::digest(&bytes)));
        write_manifest(root.path(), &m);
        assert_eq!(
            check_integrity(root.path(), &expected()),
            Err(ArtifactError::Executable),
            "{offset}"
        );
    }
}

#[cfg(unix)]
#[test]
fn executable_permissions_are_not_interchangeable_with_assets() {
    use std::os::unix::fs::PermissionsExt;
    let root = fixture();
    std::fs::set_permissions(
        root.path().join(WORKER_PATH),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert_eq!(
        check_integrity(root.path(), &expected()),
        Err(ArtifactError::Executable)
    );
    let root = fixture();
    std::fs::set_permissions(
        root.path()
            .join("Contents/Resources/Compute/assets/mlx.metallib"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert_eq!(
        check_integrity(root.path(), &expected()),
        Err(ArtifactError::Path)
    );
}

#[cfg(unix)]
#[test]
fn symlink_files_and_intermediate_directories_are_refused() {
    use std::os::unix::fs::symlink;
    for path in [
        WORKER_PATH,
        MANIFEST_PATH,
        "Contents/Resources/Compute/assets/mlx.metallib",
    ] {
        let root = fixture();
        let file = root.path().join(path);
        let target = root.path().join("target");
        std::fs::rename(&file, &target).unwrap();
        symlink(&target, &file).unwrap();
        assert_eq!(
            check_integrity(root.path(), &expected()),
            Err(ArtifactError::Path)
        );
    }
    let root = fixture();
    let dir = root.path().join("Contents/Helpers");
    let outside = tempfile::tempdir().unwrap();
    std::fs::rename(&dir, outside.path().join("helpers")).unwrap();
    symlink(outside.path().join("helpers"), &dir).unwrap();
    assert_eq!(
        check_integrity(root.path(), &expected()),
        Err(ArtifactError::Path)
    );
}
