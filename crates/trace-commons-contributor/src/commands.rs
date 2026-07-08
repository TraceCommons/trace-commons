//! CLI command implementations: login, whoami, logout, mint-grant.
//!
//! These are thin orchestration layers over `config`, `identity`, and
//! `issuer_client`. They never print raw `user_subject` (only its hash) and
//! never echo issuer response bodies on error.

use std::path::Path;

use anyhow::{Context, Result};
use trace_commons_operator_client::host_allowlist::HostAllowlist;
use trace_commons_protocol::onboarding::user_subject_hash;

use crate::config::{ContributorConfig, ConfigStore, CONTRIBUTOR_CONFIG_SCHEMA_VERSION};
use crate::identity::{build_enroll_request, mint_grant, pem_to_pkcs8_der, DeviceIdentity, EnrollmentGrant};
use crate::issuer_client::IssuerClient;

/// Build the allowlist to enforce for issuer requests: the config's
/// `allowed_hosts` CSV when set, otherwise `TRACE_COMMONS_ALLOWED_HOSTS`.
fn allowlist_for(allowed_hosts: Option<&str>) -> HostAllowlist {
    match allowed_hosts {
        Some(csv) => HostAllowlist::from_csv(csv),
        None => HostAllowlist::from_env(),
    }
}

/// Enroll this device with an instance-signed grant, or (with no grant)
/// print this device's key id so an instance operator can mint one.
///
/// When `allowed_hosts` is provided it takes precedence over the
/// `TRACE_COMMONS_ALLOWED_HOSTS` env fallback and is persisted into the
/// saved config so every later command enforces it.
pub async fn login(
    store: &ConfigStore,
    grant_b64: Option<&str>,
    allowed_hosts: Option<&str>,
) -> Result<()> {
    let device = DeviceIdentity::load_or_generate(store).context("loading device identity")?;

    let Some(grant_b64) = grant_b64 else {
        println!("device_key_id: {}", device.device_key_id);
        println!(
            "give this to your instance to mint an enrollment grant, then re-run \
             `login --grant <grant>`"
        );
        return Ok(());
    };

    let grant = EnrollmentGrant::decode(grant_b64).context("decoding enrollment grant")?;
    let req = build_enroll_request(&grant, &device).context("building enroll request")?;

    // Pre-enrollment there is no saved config yet; the flag takes
    // precedence, else fall back to the env var.
    let allowlist = allowlist_for(allowed_hosts);
    let client = IssuerClient::new(allowlist).context("building issuer client")?;
    let response = client.enroll(&grant.issuer_url, &req).await?;

    let cfg = ContributorConfig {
        schema_version: CONTRIBUTOR_CONFIG_SCHEMA_VERSION.to_string(),
        issuer_url: grant.issuer_url.clone(),
        ingest_url: response.ingest_url,
        audience: response.audience,
        tenant_id: response.tenant_id,
        instance_id: grant.attestation.instance_id.clone(),
        user_subject: grant.attestation.user_subject.clone(),
        device_key_id: response.device_key_id,
        consent_scopes: vec!["debugging_evaluation".to_string()],
        pii_filter: None,
        allowed_hosts: allowed_hosts.map(str::to_string),
    };
    store.save_config(&cfg).context("saving contributor config")?;

    println!("enrolled: tenant_id={}", cfg.tenant_id);
    println!(
        "Traces you submit carry the debugging_evaluation consent scope; secrets are removed \
         locally, PII is scrubbed server-side."
    );
    Ok(())
}

/// Print local identity: never the raw `user_subject`, only its hash.
pub fn whoami(store: &ConfigStore) -> Result<()> {
    let cfg = store
        .load_config()
        .context("loading contributor config")?
        .context("not logged in; run `login` first")?;
    let device = DeviceIdentity::load_or_generate(store).context("loading device identity")?;

    println!("instance_id: {}", cfg.instance_id);
    println!("tenant_id: {}", cfg.tenant_id);
    println!("device_key_id: {}", device.device_key_id);
    println!("user_subject_hash: {}", user_subject_hash(&cfg.user_subject));
    println!("config_dir: {}", store.dir().display());
    Ok(())
}

/// Delete all local contributor state (config, device key, receipts).
pub fn logout(store: &ConfigStore) -> Result<()> {
    store.wipe().context("wiping contributor state")?;
    println!("logged out; local state removed");
    Ok(())
}

/// Operator/dogfood tool: mint an enrollment grant with an instance private
/// key and print it (base64) to stdout.
// Arity is fixed by the plan's interface contract for this function.
#[allow(clippy::too_many_arguments)]
pub fn mint_grant_cmd(
    store: &ConfigStore,
    instance_key_pem_path: &Path,
    instance_id: &str,
    user_subject: &str,
    audience: &str,
    issuer_url: &str,
    device_key_id: Option<&str>,
    ttl_seconds: i64,
) -> Result<()> {
    let pem = std::fs::read_to_string(instance_key_pem_path)
        .with_context(|| format!("reading {}", instance_key_pem_path.display()))?;
    let der = pem_to_pkcs8_der(&pem).context("parsing instance key PEM")?;

    let device_key_id = match device_key_id {
        Some(id) => id.to_string(),
        None => {
            DeviceIdentity::load_or_generate(store)
                .context("loading device identity")?
                .device_key_id
        }
    };

    let grant = mint_grant(
        &der,
        issuer_url,
        instance_id,
        user_subject,
        audience,
        &device_key_id,
        ttl_seconds,
        chrono::Utc::now(),
    )
    .context("minting enrollment grant")?;

    println!("{}", grant.encode());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn login_rejects_issuer_host_off_allowlist_and_saves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = DeviceIdentity::load_or_generate(&store).unwrap();
        let doc =
            ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
                .unwrap();
        // Grant issuer host is 127.0.0.1; the allowlist only permits
        // api.example, so login must fail before any request is sent.
        let grant = mint_grant(
            doc.as_ref(),
            "http://127.0.0.1:9",
            "instance-1",
            "alice",
            "aud",
            &device.device_key_id,
            300,
            chrono::Utc::now(),
        )
        .unwrap();
        let err = login(&store, Some(&grant.encode()), Some("api.example"))
            .await
            .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("not on the allowed-hosts list"), "{msg}");
        // No config was persisted.
        assert!(store.load_config().unwrap().is_none());
    }
}
