//! INTEGRATION: signs, verifies, and materializes the account-bound ownership marker.

use std::path::Path;

use crate::identity::DeviceIdentity;
use crate::skill_loop::install::{
    CodexInstalledSkill, InstallMarker, InstalledSkill, SkillInstallError,
    symbolic_target_location, valid_owner_scope_sha256,
};
use crate::skill_loop::sha256;

const INSTALL_MARKER_SIGNING_CONTEXT: &[u8] = b"trace_commons.skill_install_marker.v2\n";

fn install_marker_signing_bytes(marker: &InstallMarker) -> Result<Vec<u8>, SkillInstallError> {
    let mut unsigned = marker.clone();
    unsigned.marker_sha256.clear();
    unsigned.device_signature_b64.clear();
    let encoded = serde_json::to_vec(&unsigned).map_err(|_| SkillInstallError::WriteFailed)?;
    let mut bytes = Vec::with_capacity(INSTALL_MARKER_SIGNING_CONTEXT.len() + encoded.len());
    bytes.extend_from_slice(INSTALL_MARKER_SIGNING_CONTEXT);
    bytes.extend_from_slice(&encoded);
    Ok(bytes)
}

pub(super) fn install_marker_sha256(marker: &InstallMarker) -> Result<String, SkillInstallError> {
    install_marker_signing_bytes(marker).map(|bytes| sha256(&bytes))
}

pub(super) fn sign_install_marker(
    marker: &mut InstallMarker,
    identity: &DeviceIdentity,
) -> Result<(), SkillInstallError> {
    marker.marker_sha256 = install_marker_sha256(marker)?;
    let bytes = install_marker_signing_bytes(marker)?;
    marker.device_signature_b64 = identity.sign_b64(&bytes);
    Ok(())
}

pub(super) fn canonical_marker_json(marker: &InstallMarker) -> Result<String, SkillInstallError> {
    serde_json::to_string_pretty(marker).map_err(|_| SkillInstallError::PlanChanged)
}

fn marker_has_valid_digest(marker: &InstallMarker) -> bool {
    !marker.marker_sha256.is_empty()
        && install_marker_sha256(marker).is_ok_and(|digest| digest == marker.marker_sha256)
}

pub(super) fn marker_has_valid_authentication(
    marker: &InstallMarker,
    identity: &DeviceIdentity,
    owner_scope_sha256: &str,
) -> bool {
    marker.schema_version == 2
        && valid_owner_scope_sha256(owner_scope_sha256)
        && marker.owner_scope_sha256 == owner_scope_sha256
        && marker.device_key_id == identity.device_key_id
        && marker_has_valid_digest(marker)
        && install_marker_signing_bytes(marker)
            .is_ok_and(|bytes| identity.verifies_b64(&bytes, &marker.device_signature_b64))
}

pub(super) fn marker_matches_installed(marker: &InstallMarker, installed: &InstalledSkill) -> bool {
    marker.schema_version == 2
        && marker.install_id == installed.install_id
        && marker.evaluation_id == installed.evaluation_id
        && marker.source_submission_id == installed.source_submission_id
        && marker.tool == installed.tool
        && marker.name == installed.name
        && marker.skill_sha256 == installed.skill_sha256
        && marker.marker_sha256 == installed.marker_sha256
        && marker.installed_at == installed.installed_at
}

pub(super) fn installed_from_marker(
    marker: InstallMarker,
    target: &Path,
) -> Result<CodexInstalledSkill, SkillInstallError> {
    let receipt = InstalledSkill {
        install_id: marker.install_id,
        evaluation_id: marker.evaluation_id,
        source_submission_id: marker.source_submission_id,
        tool: marker.tool,
        target_location: symbolic_target_location(&marker.name),
        name: marker.name,
        skill_sha256: marker.skill_sha256,
        marker_sha256: marker.marker_sha256,
        installed_at: marker.installed_at,
    };
    Ok(CodexInstalledSkill {
        receipt,
        local_target_path: target.to_path_buf(),
    })
}
