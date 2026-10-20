//! Control-token creation shared by embedded and command-line hosts.
use anyhow::{Context, Result};
use ironwire_core::config::PathsConfig;

/// Read the control token, minting one on first use.
///
/// Loopback alone is not authorisation on a shared machine: this surface
/// exposes the ledger and can move a user's traffic (`docs/TRUST.md` §5).
pub fn control_token(paths: &PathsConfig) -> Result<String> {
    let path = paths.control_token_file();
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    let token = mint_token()?;
    // Atomic and owner-only. A truncated token locks the user out of their own
    // daemon until they find and delete the file.
    ironwire_core::atomic::write(&path, &token)
        .with_context(|| format!("writing {}", path.display()))?;
    restrict_permissions(&path, 0o600)?;
    Ok(token)
}

/// 256 bits from the OS CSPRNG, hex-encoded.
fn mint_token() -> Result<String> {
    // OS randomness on every supported platform, with no weak fallback.
    let mut buf = [0u8; 32];
    getrandom::getrandom(&mut buf).map_err(|_| anyhow::anyhow!("OS randomness unavailable"))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(unix)]
pub fn restrict_permissions(path: &std::path::Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let permissions = std::fs::Permissions::from_mode(mode);
    std::fs::set_permissions(path, permissions)
        .with_context(|| format!("restricting permissions on {}", path.display()))
}

#[cfg(not(unix))]
pub fn restrict_permissions(path: &std::path::Path, _mode: u32) -> Result<()> {
    ironwire_ledger::token_spool::secure_windows_path(path)
        .map_err(|_| anyhow::anyhow!("private-state-permissions"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minted_tokens_are_long_and_not_repeated() {
        let a = mint_token().unwrap();
        let b = mint_token().unwrap();
        assert_eq!(a.len(), 64, "256 bits, hex-encoded");
        assert_ne!(
            a, b,
            "a predictable control token is a privilege escalation"
        );
    }
}
