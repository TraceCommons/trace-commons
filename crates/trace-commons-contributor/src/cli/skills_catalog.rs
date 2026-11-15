//! `near-ai skills-catalog`: the client half of publishing the signed Skills
//! catalog. The operator's exporter writes the payload; these commands write
//! the exact bytes a key signs, check a KMS signature against a key this
//! build pins, and verify a served document exactly as the daemon would.
//! Nothing here fetches or publishes.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::path::{Path, PathBuf};
use trace_commons_protocol::skills_catalog::{
    self as catalog, CatalogDocument, CatalogSignature, Environment, MAX_CATALOG_BYTES,
};

#[derive(Args)]
pub(super) struct CatalogArgs {
    #[command(subcommand)]
    action: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Validate an unsigned payload, then write the exact bytes a key signs.
    SigningInput {
        #[arg(long)]
        file: PathBuf,
        #[arg(long, value_parser = ["production", "staging", "testing"])]
        environment: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Check a signature against a key this build pins for the environment,
    /// then write the served document.
    Assemble {
        #[arg(long)]
        file: PathBuf,
        #[arg(long, value_parser = ["production", "staging", "testing"])]
        environment: String,
        /// The signature as `gcloud kms asymmetric-sign` wrote it.
        #[arg(long)]
        signature: PathBuf,
        /// The signing key's public key, `PUBLIC KEY` PEM.
        #[arg(long)]
        public_key_pem: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Verify a served document exactly as this build's daemon would and
    /// print a summary.
    Verify {
        #[arg(long)]
        file: PathBuf,
        #[arg(long, value_parser = ["production", "staging", "testing"])]
        environment: String,
    },
}

fn environment_of(name: &str) -> Environment {
    match name {
        "production" => Environment::Production,
        "staging" => Environment::Staging,
        _ => Environment::Testing,
    }
}

fn read_bounded(file: &Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(file)?
        .take(MAX_CATALOG_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= MAX_CATALOG_BYTES, "skills-catalog-too-large");
    Ok(bytes)
}

/// Structure and time, as a client would check them after the signature.
fn check_payload(payload: &[u8], environment: Environment) -> Result<CatalogDocument> {
    let document = catalog::decode_payload(payload, environment)?;
    anyhow::ensure!(
        document.is_current(chrono::Utc::now()),
        "skills-catalog-not-current"
    );
    Ok(document)
}

pub(super) fn signing_input(payload: &[u8], environment: Environment) -> Result<Vec<u8>> {
    check_payload(payload, environment)?;
    Ok(catalog::signing_input(payload))
}

/// The served document, refused unless `public_key` is pinned and the
/// signature verifies, and unless the result opens exactly as a client
/// opens it.
pub(super) fn assemble(
    payload: &[u8],
    environment: Environment,
    signature: &[u8],
    public_key: [u8; 32],
    pinned: &[[u8; 32]],
) -> Result<Vec<u8>> {
    check_payload(payload, environment)?;
    anyhow::ensure!(
        pinned.contains(&public_key),
        "signing-key-not-pinned-for-environment"
    );
    let signature = super::feature_flags_cli::decode_signature_file(signature)?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public_key)
        .verify(&catalog::signing_input(payload), &signature)
        .map_err(|_| anyhow::anyhow!("signature-does-not-match-public-key"))?;
    let document = catalog::assemble_signed_catalog(
        String::from_utf8(payload.to_vec()).context("payload is not UTF-8")?,
        vec![CatalogSignature {
            key_id: catalog::key_id_for(&public_key),
            signature: hex::encode(signature),
        }],
    );
    verify(&document, environment, pinned)?;
    Ok(document)
}

pub(super) fn verify(
    document: &[u8],
    environment: Environment,
    pinned: &[[u8; 32]],
) -> Result<CatalogDocument> {
    Ok(catalog::parse_signed_catalog(document, pinned, environment, chrono::Utc::now())?.0)
}

pub(super) fn run(args: &CatalogArgs) -> Result<()> {
    match &args.action {
        Action::SigningInput {
            file,
            environment,
            output,
        } => {
            let bytes = signing_input(&read_bounded(file)?, environment_of(environment))?;
            std::fs::write(output, bytes)?;
        }
        Action::Assemble {
            file,
            environment,
            signature,
            public_key_pem,
            output,
        } => {
            let environment = environment_of(environment);
            let public_key = super::feature_flags_cli::public_key_from_pem(
                &std::fs::read_to_string(public_key_pem)?,
            )?;
            let document = assemble(
                &read_bounded(file)?,
                environment,
                &std::fs::read(signature)?,
                public_key,
                &crate::skills_catalog::pinned_keys(environment)?,
            )?;
            std::fs::write(output, document)?;
        }
        Action::Verify { file, environment } => {
            let environment = environment_of(environment);
            let document = verify(
                &read_bounded(file)?,
                environment,
                &crate::skills_catalog::pinned_keys(environment)?,
            )?;
            println!(
                "{}",
                serde_json::json!({
                    "environment": environment.as_str(),
                    "revision": document.revision,
                    "issued_at": document.issued_at,
                    "expires_at": document.expires_at,
                    "publishers": document.publishers.len(),
                    "skills": document.skills.len(),
                })
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use ring::signature::{Ed25519KeyPair, KeyPair as _};

    /// A throwaway key for this test run only. Never a real catalog key.
    fn key() -> Ed25519KeyPair {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
    }

    fn payload() -> Vec<u8> {
        let issued = chrono::DateTime::from_timestamp(Utc::now().timestamp(), 0).unwrap();
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 1, "checks_version": 1, "environment": "staging",
            "revision": issued.timestamp(), "issued_at": issued,
            "expires_at": issued + Duration::days(7),
            "publishers": [], "skills": []
        }))
        .unwrap()
    }

    #[test]
    fn assemble_and_verify_round_trip_only_with_a_pinned_key() {
        let signer = key();
        let public: [u8; 32] = signer.public_key().as_ref().try_into().unwrap();
        let payload = payload();
        let input = signing_input(&payload, Environment::Staging).unwrap();
        let signature = signer.sign(&input);
        let document = assemble(
            &payload,
            Environment::Staging,
            signature.as_ref(),
            public,
            &[public],
        )
        .unwrap();
        assert_eq!(
            verify(&document, Environment::Staging, &[public])
                .unwrap()
                .skills
                .len(),
            0
        );
        assert!(verify(&document, Environment::Production, &[public]).is_err());
        assert!(verify(&document, Environment::Staging, &[]).is_err());
        let unpinned = assemble(
            &payload,
            Environment::Staging,
            signature.as_ref(),
            public,
            &[],
        );
        assert_eq!(
            unpinned.unwrap_err().to_string(),
            "signing-key-not-pinned-for-environment"
        );
        let wrong = key().sign(&input);
        assert!(
            assemble(
                &payload,
                Environment::Staging,
                wrong.as_ref(),
                public,
                &[public]
            )
            .is_err()
        );
    }

    #[test]
    fn signing_input_refuses_a_payload_for_another_environment() {
        assert!(signing_input(&payload(), Environment::Production).is_err());
        assert!(signing_input(b"{}", Environment::Staging).is_err());
    }
}
