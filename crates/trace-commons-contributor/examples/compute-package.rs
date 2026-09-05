//! Inert developer assembly. Does not sign, launch, or modify an existing app.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};
use trace_commons_contributor::compute::artifact::{
    ArtifactExpectation, MANIFEST_PATH, WORKER_PATH, check_integrity,
};

const MAX_FILE: u64 = 512 * 1024 * 1024;
const ASSET_PATH: &str = "Contents/Resources/Compute/assets/mlx.metallib";
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

// A bounded streaming copy; hash the bytes actually written, not a prior read.
fn copy_pin(source: &Path, destination: &Path, executable: bool) -> Result<Value> {
    if !source.is_absolute() || !fs::symlink_metadata(source)?.file_type().is_file() {
        return Err("compute-package-source-invalid".into());
    }
    // Reject symlinked parent components too. This is not a race-proof sandbox.
    for parent in source.ancestors().skip(1) {
        if fs::symlink_metadata(parent)?.file_type().is_symlink() {
            return Err("compute-package-source-invalid".into());
        }
    }
    let input = fs::File::open(source)?;
    let size = input.metadata()?.len();
    if size == 0 || size > MAX_FILE {
        return Err("compute-package-source-size-invalid".into());
    }
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let mut input = input.take(MAX_FILE + 1);
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 32768];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > MAX_FILE {
            return Err("compute-package-source-size-invalid".into());
        }
        output.write_all(&buffer[..count])?;
        hash.update(&buffer[..count]);
    }
    if total != size {
        return Err("compute-package-source-changed".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        output.set_permissions(fs::Permissions::from_mode(if executable {
            0o755
        } else {
            0o644
        }))?;
    }
    #[cfg(not(unix))]
    let _ = executable;
    output.sync_all()?;
    Ok(json!({"size_bytes": total, "sha256": hex::encode(hash.finalize())}))
}

fn assemble(
    bundle: &Path,
    worker: &Path,
    metal: &Path,
    expected: &ArtifactExpectation<'_>,
) -> Result<()> {
    if !bundle.is_absolute() || bundle.file_name().is_none() {
        return Err("compute-package-destination-invalid".into());
    }
    for parent in bundle.ancestors().skip(1) {
        if !fs::symlink_metadata(parent)?.file_type().is_dir() {
            return Err("compute-package-destination-invalid".into());
        }
    }
    // Exclusively reserve a fresh directory. On failure retain partial output for
    // inspection; never recursively remove a caller-specified directory.
    fs::create_dir(bundle)?;
    fs::create_dir_all(bundle.join("Contents/Helpers"))?;
    fs::create_dir_all(bundle.join("Contents/Resources/Compute/assets"))?;
    let worker = copy_pin(worker, &bundle.join(WORKER_PATH), true)?;
    let mut asset = copy_pin(metal, &bundle.join(ASSET_PATH), false)?;
    asset["relative_path"] = json!("mlx.metallib");
    let manifest = json!({
        "schema_version": 1, "source_revision": expected.source_revision,
        "target": "aarch64-apple-darwin", "backend": "mlx",
        "minimum_macos": [15, 0, 0], "ipc_version": 0,
        "compatibility_id": expected.compatibility_id,
        "signing_identifier": expected.signing_identifier,
        "signing_team": expected.signing_team,
        "worker": worker, "assets": [asset]
    });
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(bundle.join(MANIFEST_PATH))?;
    output.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
    output.sync_all()?;
    check_integrity(bundle, expected)?;
    Ok(())
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 7 {
        return Err("usage: compute-package ABS_NEW_BUNDLE ABS_WORKER ABS_METALLIB SOURCE_REV COMPAT_ID SIGNING_ID TEAM_ID".into());
    }
    let expected = ArtifactExpectation {
        source_revision: &args[3],
        compatibility_id: &args[4],
        signing_identifier: &args[5],
        signing_team: &args[6],
        host_target: "aarch64-apple-darwin",
        host_macos: [15, 0, 0],
    };
    assemble(
        Path::new(&args[0]),
        Path::new(&args[1]),
        Path::new(&args[2]),
        &expected,
    )?;
    println!(
        "compute-package-integrity-checked signature_verified=false provenance_verified=false launch_authorized=false"
    );
    Ok(())
}

fn main() {
    if run().is_err() {
        // Do not print OS errors containing private source or destination paths.
        eprintln!(
            "compute-package-failed; requires ABS_NEW_BUNDLE ABS_WORKER ABS_METALLIB SOURCE_REV COMPAT_ID SIGNING_ID TEAM_ID; partial output may remain"
        );
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expected() -> ArtifactExpectation<'static> {
        ArtifactExpectation {
            source_revision: "1111111111111111111111111111111111111111",
            compatibility_id: "test-v1",
            signing_identifier: "org.example.worker",
            signing_team: "TESTTEAM01",
            host_target: "aarch64-apple-darwin",
            host_macos: [15, 0, 0],
        }
    }
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let header: Vec<u8> = [
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
        .collect();
        fs::write(dir.path().join("worker"), header).unwrap();
        fs::write(dir.path().join("metal"), b"fixture-not-real-metal").unwrap();
        dir
    }
    #[test]
    fn assembly_preserves_bytes_and_refuses_overwrite() {
        let dir = fixture();
        let root = dir.path().canonicalize().unwrap();
        let bundle = root.join("Pilot.app");
        assemble(
            &bundle,
            &root.join("worker"),
            &root.join("metal"),
            &expected(),
        )
        .unwrap();
        assert_eq!(
            fs::read(bundle.join(WORKER_PATH)).unwrap(),
            fs::read(root.join("worker")).unwrap()
        );
        assert!(
            assemble(
                &bundle,
                &root.join("worker"),
                &root.join("metal"),
                &expected()
            )
            .is_err()
        );
        check_integrity(&bundle, &expected()).unwrap();
        fs::write(bundle.join(ASSET_PATH), b"tampered").unwrap();
        assert!(check_integrity(&bundle, &expected()).is_err());
    }
    #[test]
    fn existing_destination_is_untouched() {
        let dir = fixture();
        let root = dir.path().canonicalize().unwrap();
        let bundle = root.join("Existing.app");
        fs::create_dir(&bundle).unwrap();
        fs::write(bundle.join("sentinel"), b"preserve-me").unwrap();
        let permissions = fs::metadata(&bundle).unwrap().permissions();
        assert!(
            assemble(
                &bundle,
                &root.join("worker"),
                &root.join("metal"),
                &expected()
            )
            .is_err()
        );
        assert_eq!(fs::read(bundle.join("sentinel")).unwrap(), b"preserve-me");
        assert_eq!(fs::read_dir(&bundle).unwrap().count(), 1);
        assert_eq!(fs::metadata(&bundle).unwrap().permissions(), permissions);
    }
    #[test]
    fn refuses_invalid_executable_and_metadata() {
        let dir = fixture();
        let root = dir.path().canonicalize().unwrap();
        let mut policy = expected();
        policy.source_revision = "not-a-revision";
        assert!(
            assemble(
                &root.join("Invalid.app"),
                &root.join("worker"),
                &root.join("metal"),
                &policy
            )
            .is_err()
        );
        fs::write(root.join("worker"), b"not-MachO").unwrap();
        assert!(
            assemble(
                &root.join("InvalidCode.app"),
                &root.join("worker"),
                &root.join("metal"),
                &expected()
            )
            .is_err()
        );
    }
    #[test]
    fn refuses_empty_and_oversized_sources() {
        let dir = fixture();
        let root = dir.path().canonicalize().unwrap();
        for (name, size) in [("empty", 0), ("oversized", MAX_FILE + 1)] {
            fs::File::create(root.join(name))
                .unwrap()
                .set_len(size)
                .unwrap();
            assert!(copy_pin(&root.join(name), &root.join(format!("{name}-copy")), false).is_err());
        }
    }
    #[cfg(unix)]
    #[test]
    fn refuses_symlinks_and_non_regular_sources() {
        use std::os::unix::fs::symlink;
        let dir = fixture();
        let root = dir.path().canonicalize().unwrap();
        symlink(root.join("worker"), root.join("link")).unwrap();
        symlink(&root, root.join("parent-link")).unwrap();
        assert!(
            assemble(
                &root.join("parent-link/Rejected.app"),
                &root.join("worker"),
                &root.join("metal"),
                &expected()
            )
            .is_err()
        );
        assert!(!root.join("Rejected.app").exists());
        for source in [
            root.join("link"),
            root.join("parent-link/worker"),
            root.clone(),
        ] {
            assert!(copy_pin(&source, &root.join("copy"), true).is_err());
        }
    }
}
