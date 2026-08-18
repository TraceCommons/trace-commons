//! The Windows code-signature check, held to `scripts/install.ps1`'s standard.
//!
//! That script is the reference for what "verified" means on Windows and the
//! two must not diverge, so this asks the same question of the same platform
//! verifier: is the Authenticode status exactly `Valid`, and does the signing
//! subject name our organisation. Checking the signer, and not merely that a
//! signature is present, is the part that matters -- a validly signed binary
//! from an unexpected publisher is exactly the case a status-only check waves
//! through.
//!
//! Matching on the organisation rather than a full distinguished name is also
//! `install.ps1`'s choice: Azure Trusted Signing issues a fresh certificate
//! per signing job, so the leaf changes constantly while `O=` does not.
//!
//! Off Windows there is no Authenticode trust provider to call, so `verify`
//! does not exist on this platform at all -- there is no stand-in function
//! that would read as "checked" when it is not. What stands in its place on
//! unix is the manifest's Ed25519 signature (`update::manifest`) plus the
//! sha256 digest pinned in that signed manifest (`update::fetch`): the
//! authority that says "this build is the one to install" and the integrity
//! check that says "these are its exact bytes" are both already mandatory
//! there, and neither depends on a platform code-signing service.

/// The substring the signing subject must contain. Kept identical to
/// `$ExpectedSigner` in `scripts/install.ps1`.
pub const EXPECTED_SIGNER: &str = "O=Iqlusion Inc";

#[derive(Debug, thiserror::Error)]
pub enum AuthenticodeError {
    /// The platform verifier could not be run at all. Fail closed: an
    /// unverifiable artifact is refused, never installed.
    #[error("update_authenticode_verifier_unavailable")]
    VerifierUnavailable,
    /// The verifier ran but its output was not the two lines expected.
    #[error("update_authenticode_malformed_verifier_output")]
    MalformedVerifierOutput,
    /// The signature is absent, or its status is anything other than `Valid`.
    #[error("update_authenticode_not_valid")]
    NotValid,
    /// A valid signature, from somebody who is not us.
    #[error("update_authenticode_unexpected_signer")]
    UnexpectedSigner,
}

/// Decide from a status and a subject. Separated from the platform call so
/// the decision is testable on every platform, including from the shared
/// conformance fixtures.
pub fn interpret(status: &str, subject: &str) -> Result<(), AuthenticodeError> {
    if status.trim() != "Valid" {
        return Err(AuthenticodeError::NotValid);
    }
    if !subject.contains(EXPECTED_SIGNER) {
        return Err(AuthenticodeError::UnexpectedSigner);
    }
    Ok(())
}

#[cfg(any(windows, test))]
fn parse_verifier_output(bytes: &[u8]) -> Result<(), AuthenticodeError> {
    let stdout =
        std::str::from_utf8(bytes).map_err(|_| AuthenticodeError::MalformedVerifierOutput)?;
    let mut status = None;
    let mut subject = None;
    for line in stdout.lines() {
        // Windows PowerShell can prefix redirected output with a UTF-8 BOM.
        let line = line.trim_start_matches('\u{feff}');
        if let Some(rest) = line.strip_prefix("STATUS=") {
            status = Some(rest.trim());
        } else if let Some(rest) = line.strip_prefix("SUBJECT=") {
            subject = Some(rest.trim());
        }
    }
    match (status, subject) {
        (Some(status), Some(subject)) => interpret(status, subject),
        _ => Err(AuthenticodeError::MalformedVerifierOutput),
    }
}

/// Verify the Authenticode signature on `path` using the platform verifier.
#[cfg(windows)]
pub fn verify(path: &std::path::Path) -> Result<(), AuthenticodeError> {
    use std::process::Command;

    // An absolute path under %SystemRoot%, never a bare `powershell.exe`: a
    // PATH lookup for the program that decides whether a binary is trusted
    // is a lookup an attacker who can write to PATH gets to answer.
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    let shell =
        std::path::Path::new(&system_root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");

    // Single-quote the path for PowerShell and double any embedded quote.
    // `-LiteralPath` additionally stops PowerShell treating `[` and `]` in a
    // staging path as wildcards.
    let quoted = format!("'{}'", path.to_string_lossy().replace('\'', "''"));
    let script = format!(
        "$ErrorActionPreference='Stop'; \
         $s = Get-AuthenticodeSignature -LiteralPath {quoted}; \
         $subject = if ($s.SignerCertificate) {{ $s.SignerCertificate.Subject }} else {{ '' }}; \
         $nl = [Environment]::NewLine; \
         $text = 'STATUS=' + $s.Status + $nl + 'SUBJECT=' + $subject + $nl; \
         $bytes = [System.Text.Encoding]::UTF8.GetBytes($text); \
         $stdout = [Console]::OpenStandardOutput(); \
         $stdout.Write($bytes, 0, $bytes.Length); \
         $stdout.Flush()"
    );

    let output = Command::new(&shell)
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|_| AuthenticodeError::VerifierUnavailable)?;
    if !output.status.success() {
        // stderr is deliberately not surfaced: it would carry the staging
        // path.
        return Err(AuthenticodeError::VerifierUnavailable);
    }

    parse_verifier_output(&output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_valid_signature_from_our_organisation_is_accepted() {
        assert!(
            interpret(
                "Valid",
                "CN=Iqlusion Inc, O=Iqlusion Inc, L=San Francisco, S=California, C=US"
            )
            .is_ok()
        );
    }

    #[test]
    fn an_unsigned_binary_is_refused() {
        assert!(matches!(
            interpret("NotSigned", "").unwrap_err(),
            AuthenticodeError::NotValid
        ));
    }

    #[test]
    fn verifier_output_accepts_utf8_with_or_without_a_bom() {
        for output in [
            b"STATUS=NotSigned\r\nSUBJECT=\r\n".as_slice(),
            b"\xef\xbb\xbfSTATUS=NotSigned\r\nSUBJECT=\r\n".as_slice(),
        ] {
            assert!(matches!(
                parse_verifier_output(output).unwrap_err(),
                AuthenticodeError::NotValid
            ));
        }
    }

    #[test]
    fn every_non_valid_status_is_refused() {
        // Status-only checks that accept anything other than exactly "Valid"
        // are how a revoked or untrusted-root signature gets waved through.
        for status in ["HashMismatch", "UnknownError", "NotTrusted", "Incompatible"] {
            assert!(matches!(
                interpret(status, "O=Iqlusion Inc").unwrap_err(),
                AuthenticodeError::NotValid
            ));
        }
    }

    #[test]
    fn a_valid_signature_from_the_wrong_publisher_is_refused() {
        // This is the case a status-only check waves through, and it is the
        // one that matters: a validly signed binary from somebody else.
        assert!(matches!(
            interpret("Valid", "CN=Contoso Ltd, O=Contoso Ltd, C=US").unwrap_err(),
            AuthenticodeError::UnexpectedSigner
        ));
    }

    #[test]
    fn a_valid_status_with_no_subject_is_refused() {
        assert!(matches!(
            interpret("Valid", "").unwrap_err(),
            AuthenticodeError::UnexpectedSigner
        ));
    }

    #[test]
    fn the_status_comparison_ignores_surrounding_whitespace_only() {
        assert!(interpret(" Valid \r", " O=Iqlusion Inc ").is_ok());
        assert!(matches!(
            interpret("valid", "O=Iqlusion Inc").unwrap_err(),
            AuthenticodeError::NotValid
        ));
    }

    #[test]
    fn the_expected_signer_matches_install_ps1() {
        // scripts/install.ps1 pins $ExpectedSigner = 'O=Iqlusion Inc'. The
        // two must not drift.
        assert_eq!(EXPECTED_SIGNER, "O=Iqlusion Inc");
    }
}
