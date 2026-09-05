//! Read-only developer integrity check, NOT signature verification or launch.
use std::path::Path;
use trace_commons_contributor::compute::artifact::{ArtifactExpectation, check_integrity};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 6 {
        return Err(
            "usage: compute-artifact ABS_BUNDLE SOURCE_REV COMPAT_ID SIGNING_ID TEAM_ID MANIFEST_SHA256".into(),
        );
    }
    // This harness checks a declared macOS 15 arm64 target, not machine eligibility.
    let manifest_sha256: [u8; 32] = hex::decode(&args[5])?
        .try_into()
        .map_err(|_| "manifest digest must contain 32 bytes")?;
    let expected = ArtifactExpectation {
        manifest_sha256,
        source_revision: &args[1],
        compatibility_id: &args[2],
        signing_identifier: &args[3],
        signing_team: &args[4],
        host_target: "aarch64-apple-darwin",
        host_macos: [15, 0, 0],
    };
    let checked = check_integrity(Path::new(&args[0]), &expected)?;
    println!(
        "integrity-only assets={} bytes={} signature_verified=false launch_authorized=false",
        checked.asset_count, checked.checked_bytes
    );
    Ok(())
}
